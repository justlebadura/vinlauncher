//! Capa de red P2P: listener, handshake autenticado por token y conexión.
//!
//! Estrategia de conexión entre dos internets distintas (sin servidor):
//! 1. Dial directo a la IP:puerto del código de invitación. Si el peer tiene
//!    puertos abiertos / UPnP, funciona de inmediato.
//! 2. En LAN, el dial directo a la IP local funciona siempre.
//!
//! El protocolo sobre TCP es: mensajes JSON delimitados por `\n` (handshake),
//! luego el canal de transferencia (ver `transfer`).

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};

use crate::identity::FriendIdentity;

/// Puerto por defecto para el listener P2P.
pub const DEFAULT_PORT: u16 = 45600;

/// Timeout total para establecer una conexión P2P.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Timeout de una sola línea de handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Primer mensaje del handshake (JSON por línea).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Handshake {
    pub v: u32,
    pub token: String,
    pub identity: FriendIdentity,
}

/// Error de red P2P.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("timeout al conectar (¿está vinlauncher abierto y en línea?)")]
    Timeout,
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("handshake rechazado: token incorrecto")]
    BadToken,
    #[error("conexión cerrada durante el handshake")]
    Closed,
}

/// Abre el listener TCP local y devuelve (puerto_asignado, listener).
pub async fn bind_listener(preferred_port: u16) -> std::io::Result<(u16, TcpListener)> {
    // Intenta el puerto preferido; si está ocupado, deja que el SO asigne uno.
    let listener = match TcpListener::bind(("0.0.0.0", preferred_port)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    Ok((port, listener))
}

/// Escribe un handshake al stream.
pub async fn send_handshake(
    writer: &mut OwnedWriteHalf,
    token: &str,
    identity: &FriendIdentity,
) -> std::io::Result<()> {
    let hs = Handshake {
        v: 1,
        token: token.to_string(),
        identity: identity.clone(),
    };
    let mut line = serde_json::to_string(&hs).expect("handshake serializes");
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Comparación en tiempo constante para tokens.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Lee y verifica el handshake entrante contra el token esperado (si se da).
pub async fn read_handshake(
    reader: &mut BufReader<OwnedReadHalf>,
    expected_token: Option<&str>,
) -> Result<Handshake, NetError> {
    let mut line = String::new();
    // 64 KiB máx por línea de handshake: suficiente y evita abusos.
    let read_fut = reader.read_line(&mut line);
    let n = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_fut)
        .await
        .map_err(|_| NetError::Timeout)?;
    let n = n?;
    if n == 0 {
        return Err(NetError::Closed);
    }
    let hs: Handshake =
        serde_json::from_str(line.trim()).map_err(|_| NetError::Closed)?;
    if let Some(expected) = expected_token {
        if !constant_time_eq(&hs.token, expected) {
            return Err(NetError::BadToken);
        }
    }
    Ok(hs)
}

/// Establece conexión P2P con un amigo y hace el handshake saliente.
/// Devuelve las mitades de lectura/escritura ya autenticadas.
pub async fn connect_to_peer(
    their_endpoint: &str,
    my_identity: &FriendIdentity,
    token: &str,
) -> Result<(OwnedReadHalf, OwnedWriteHalf, Handshake), NetError> {
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(their_endpoint))
            .await
            .map_err(|_| NetError::Timeout)??;
        stream.set_nodelay(true)?;
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        send_handshake(&mut write, token, my_identity).await?;
        let hs = read_handshake(&mut reader, None).await?;
        let read = reader.into_inner();
        Ok((read, write, hs))
    })
    .await
    .map_err(|_| NetError::Timeout)?
}

/// Acepta una conexión entrante verificando el token (si se da).
/// Devuelve las mitades de lectura/escritura ya autenticadas.
pub async fn accept_connection(
    stream: TcpStream,
    expected_token: Option<&str>,
) -> Result<(OwnedReadHalf, OwnedWriteHalf, Handshake), NetError> {
    stream.set_nodelay(true)?;
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let hs = read_handshake(&mut reader, expected_token).await?;
    let read = reader.into_inner();
    Ok((read, write, hs))
}
