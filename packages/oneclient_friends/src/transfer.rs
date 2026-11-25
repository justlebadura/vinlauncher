//! Transferencia de archivos/instancias sobre la conexión P2P.
//!
//! Protocolo (después del handshake, sobre el mismo TCP):
//! ```text
//! {"action":"file","name":"...","size":N,"sha1":"..."}\n
//! [N bytes crudos]
//! ```
//! El receptor valida el tamaño y el sha1 antes de mover el archivo a la
//! bandeja de entrada (`inbox_dir`), donde la UI lo puede importar como
//! cluster (`.mrpack` se importa con el flujo existente de modpacks).

use std::path::{Path, PathBuf};

use sha1::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

/// Acción: transferir un archivo (instancia empaquetada .mrpack o manifiesto).
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct FileHeader {
    pub action: String, // "file"
    pub name: String,
    pub size: u64,
    pub sha1: String,
}

/// Tamaño máximo de transferencia: 4 GiB.
const MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("el archivo excede el tamaño máximo")]
    TooLarge,
    #[error("hash sha1 no coincide: el archivo llegó corrupto")]
    HashMismatch,
    #[error("nombre de archivo inválido")]
    BadName,
}

fn sanitize_name(name: &str) -> Option<String> {
    let clean: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .collect();
    if clean.is_empty() || clean.starts_with('.') {
        None
    } else {
        Some(clean)
    }
}

/// Envía un "ping" de keep-alive: solo cabecera y cierre. El receptor que
/// esperaba un archivo ve `action == "ping"` y sabe que es solo señales.
pub async fn send_ping(writer: &mut OwnedWriteHalf) -> Result<(), TransferError> {
    let header = FileHeader {
        action: "ping".into(),
        name: String::new(),
        size: 0,
        sha1: String::new(),
    };
    let mut line = serde_json::to_string(&header).expect("header serializes");
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Envía un archivo por la conexión. `name` es el nombre visible para el receptor.
pub async fn send_file(
    writer: &mut OwnedWriteHalf,
    path: &Path,
    display_name: &str,
) -> Result<(), TransferError> {
    let data = tokio::fs::read(path).await?;
    if data.len() as u64 > MAX_FILE_SIZE {
        return Err(TransferError::TooLarge);
    }
    let mut hasher = sha1::Sha1::new();
    hasher.update(&data);
    let sha1 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let header = FileHeader {
        action: "file".into(),
        name: display_name.to_string(),
        size: data.len() as u64,
        sha1,
    };
    let mut line = serde_json::to_string(&header).expect("header serializes");
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(&data).await?;
    writer.flush().await?;
    Ok(())
}

/// Recibe un archivo de la conexión y lo escribe en `inbox_dir`.
/// Devuelve la ruta destino y el nombre recibido.
pub async fn receive_file(
    reader: &mut OwnedReadHalf,
    inbox_dir: &Path,
) -> Result<(PathBuf, FileHeader), TransferError> {
    let header = read_header(reader).await?;
    let dest = write_received(reader, inbox_dir, &header).await?;
    Ok((dest, header))
}

/// Lo que puede llegar tras el handshake.
#[derive(Debug)]
pub enum Incoming {
    /// Llegó un archivo (ruta destino, cabecera).
    File(PathBuf, FileHeader),
    /// Solo un ping de keep-alive ("estoy aquí / soy reachable").
    Ping,
}

/// Lee la cabecera y despacha: archivo → lo guarda; ping → devuelve `Ping`.
pub async fn receive_any(
    reader: &mut OwnedReadHalf,
    inbox_dir: &Path,
) -> Result<Incoming, TransferError> {
    let header = read_header(reader).await?;
    if header.action == "ping" {
        return Ok(Incoming::Ping);
    }
    let dest = write_received(reader, inbox_dir, &header).await?;
    Ok(Incoming::File(dest, header))
}

pub(crate) async fn read_header(reader: &mut OwnedReadHalf) -> Result<FileHeader, TransferError> {    // Lee la línea de cabecera byte a byte hasta \n.
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let n = reader.read(&mut byte).await?;
        if n == 0 {
            return Err(TransferError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > 64 * 1024 {
            return Err(TransferError::BadName);
        }
    }
    serde_json::from_slice(&line).map_err(|_| TransferError::BadName)
}

pub(crate) async fn write_received(
    reader: &mut OwnedReadHalf,
    inbox_dir: &Path,
    header: &FileHeader,
) -> Result<PathBuf, TransferError> {
    if header.size > MAX_FILE_SIZE {
        return Err(TransferError::TooLarge);
    }
    let safe_name = sanitize_name(&header.name).ok_or(TransferError::BadName)?;

    let mut hasher = sha1::Sha1::new();
    let mut remaining = header.size;
    let tmp_path = inbox_dir.join(format!("{safe_name}.part"));
    {
        use tokio::io::AsyncWriteExt as _;
        let mut out = tokio::fs::File::create(&tmp_path).await?;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = reader.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(TransferError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        out.flush().await?;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if digest != header.sha1 {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(TransferError::HashMismatch);
    }
    let final_path = unique_inbox_path(inbox_dir, &safe_name);
    tokio::fs::rename(&tmp_path, &final_path).await?;
    Ok(final_path)
}

/// Evita sobrescribir archivos previos en la bandeja: nombre (1).ext, (2)…
fn unique_inbox_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = Path::new(name).extension().and_then(|s| s.to_str());
    for i in 1..10_000u32 {
        let numbered = match ext {
            Some(e) => dir.join(format!("{stem} ({i}).{e}")),
            None => dir.join(format!("{stem} ({i})")),
        };
        if !numbered.exists() {
            return numbered;
        }
    }
    dir.join(format!("{stem}-{}", std::process::id()))
}
