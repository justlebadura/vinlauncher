//! Códigos de invitación: la única "coordenada" que hay que intercambiar.
//!
//! Un código contiene: identidad, puerto de escucha, IP pública detectada y
//! un token secreto aleatorio. El token evita conexiones de desconocidos que
//! descubran la IP:puerto por otros medios.

use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::identity::FriendIdentity;

/// Versión del formato de código.
const INVITE_VERSION: u32 = 1;

/// Contenido de un código de invitación (se codifica en base64-url).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitePayload {
    pub v: u32,
    pub identity: FriendIdentity,
    /// Puerto TCP donde el emisor escucha.
    pub listen_port: u16,
    /// IP pública detectada del emisor, si se pudo resolver.
    #[serde(default)]
    pub public_ip: Option<String>,
    /// Token compartido: solo quien lo tenga puede completar el handshake.
    pub token: String,
}

/// Error de decodificación de códigos.
#[derive(Debug, thiserror::Error)]
pub enum InviteError {
    #[error("código de invitación inválido o corrupto")]
    Invalid,
    #[error("versión de código de invitación no soportada: {0}")]
    Version(u32),
}

/// Genera un token aleatorio de 16 bytes en hex.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Serializa un código a texto (base64-url sin padding).
pub fn encode_invite(payload: &InvitePayload) -> String {
    let json = serde_json::to_vec(payload).expect("invite payload serializes");
    let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json);
    // Prefijo legible para que el usuario reconozca el código.
    format!("VIN1-{text}")
}

/// Decodifica un código de invitación desde texto.
pub fn decode_invite(code: &str) -> Result<InvitePayload, InviteError> {
    let trimmed = code.trim();
    let body = trimmed
        .strip_prefix("VIN1-")
        .ok_or(InviteError::Invalid)?;
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body.trim())
        .map_err(|_| InviteError::Invalid)?;
    let payload: InvitePayload =
        serde_json::from_slice(&json).map_err(|_| InviteError::Invalid)?;
    if payload.v != INVITE_VERSION {
        return Err(InviteError::Version(payload.v));
    }
    Ok(payload)
}

/// Detecta la IP pública del usuario consultando un servicio de eco
/// (no es un servidor nuestro: solo devuelve la IP de salida).
pub async fn detect_public_ip() -> Option<String> {
    for url in ["https://api.ipify.org", "https://ifconfig.me/ip"] {
        match reqwest::get(url).await {
            Ok(resp) => match resp.text().await {
                Ok(text) => {
                    let ip = text.trim().to_string();
                    if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_ok() {
                        return Some(ip);
                    }
                }
                Err(e) => tracing::debug!(url, error = %e, "ip echo body failed"),
            },
            Err(e) => tracing::debug!(url, error = %e, "ip echo failed"),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let payload = InvitePayload {
            v: INVITE_VERSION,
            identity: FriendIdentity::new("069a79f4-44e9-4726-a5be-fca90e38aaf5".into(), "Notch".into()),
            listen_port: 45600,
            public_ip: Some("203.0.113.7".into()),
            token: generate_token(),
        };
        let code = encode_invite(&payload);
        let decoded = decode_invite(&code).unwrap();
        assert_eq!(decoded.identity, payload.identity);
        assert_eq!(decoded.listen_port, 45600);
        assert_eq!(decoded.token, payload.token);
    }
}
