//! vinlauncher: rebrands Polyfrost client mods (OneConfig / OneClient) inside the
//! jars that land in a cluster's mods folder.
//!
//! The cached artifact copies keep their original bytes (and their SHA-1), so
//! every download/integrity check still passes; the rebrand only applies to the
//! materialized copy the game actually loads.

use std::path::Path;

use async_zip::base::read::seek::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures_lite::AsyncReadExt;

use crate::error::{ContentError, ContentResult};

/// Display names patched inside mod resources. Lowercase ids (package ids,
/// lang keys, entrypoint names) are intentionally left alone so mod
/// dependencies and configs keep working.
const NAME_PATCHES: &[(&str, &str)] = &[("OneConfig", "VinConfig"), ("OneClient", "VinClient")];

/// Resource files whose *values* carry visible branding.
fn is_brandable_entry(name: &str) -> bool {
    let name = name.trim_start_matches('/');
    name == "fabric.mod.json"
        || name == "quilt.mod.json"
        || name == "META-INF/mods.toml"
        || (name.contains("/lang/") && name.ends_with(".json"))
}

/// Whether the given name is a nested jar shipped inside another mod jar.
fn is_nested_jar(name: &str) -> bool {
    name.starts_with("META-INF/jars/") && name.ends_with(".jar")
}

fn patch_text(bytes: &[u8]) -> Option<Vec<u8>> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return None;
    };
    let mut patched = None;
    for (from, to) in NAME_PATCHES {
        if text.contains(from) {
            patched = Some(patched.unwrap_or_else(|| text.to_string()).replace(from, to));
        }
    }
    patched.map(String::into_bytes)
}

/// Rewrites a jar's brandable resources, recursing into nested jars. Returns
/// the patched bytes, or `None` when nothing needed changing.
fn patch_jar_bytes(bytes: Vec<u8>) -> ContentResult<Option<Vec<u8>>> {
        let mut reader = futures_lite::future::block_on(ZipFileReader::new(futures_lite::io::Cursor::new(
            bytes,
        )))
        .map_err(zip_err)?;

    let entries = reader.file().entries().to_vec();
    let mut contents: Vec<(String, Vec<u8>)> = Vec::with_capacity(entries.len());
    let mut changed = false;

    for index in 0..entries.len() {
        let name = entries[index]
            .filename()
            .as_bytes()
            .iter()
            .map(|&b| b as char)
            .collect::<String>();
        if name.ends_with('/') {
            continue;
        }

        let mut data = Vec::new();
        let mut entry_reader = futures_lite::future::block_on(reader.reader_without_entry(index))
            .map_err(zip_err)?;
        futures_lite::future::block_on(entry_reader.read_to_end(&mut data))
            .map_err(std::io::Error::other)?;

        let patched = if is_brandable_entry(&name) {
            patch_text(&data)
        } else if is_nested_jar(&name) {
            match patch_jar_bytes(data.clone())? {
                Some(patched) => Some(patched),
                None => None,
            }
        } else {
            None
        };

        if let Some(patched) = patched {
            changed = true;
            contents.push((name, patched));
        } else {
            contents.push((name, data));
        }
    }

    if !changed {
        return Ok(None);
    }

    let mut out: Vec<u8> = Vec::new();
    let mut writer = ZipFileWriter::new(futures_lite::io::Cursor::new(&mut out));
    for (name, data) in contents {
        let builder = ZipEntryBuilder::new(name.into(), Compression::Deflate);
        futures_lite::future::block_on(writer.write_entry_whole(builder, &data))
            .map_err(zip_err)?;
    }
    futures_lite::future::block_on(writer.close()).map_err(zip_err)?;

    Ok(Some(out))
}

fn zip_err(err: async_zip::error::ZipError) -> ContentError {
    ContentError::StdIo(std::io::Error::other(format!("zip: {err}")))
}

/// Patches branding resources of a mod jar in place. Returns whether the file
/// was rewritten.
pub async fn patch_mod_jar(path: &Path) -> ContentResult<bool> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(ContentError::StdIo)?;

    let Some(patched) = patch_jar_bytes(bytes)? else {
        return Ok(false);
    };

    let tmp = path.with_extension("jar.vin-tmp");
    tokio::fs::write(&tmp, &patched).await.map_err(ContentError::StdIo)?;
    match tokio::fs::rename(&tmp, path).await {
        Ok(()) => Ok(true),
        Err(err) => {
            tokio::fs::remove_file(&tmp).await.ok();
            Err(ContentError::StdIo(err))
        }
    }
}
