use serde::{Deserialize, Serialize};

/// Identidad pública de un usuario de vinlauncher.
/// Se deriva de la cuenta de Minecraft activa (UUID + nombre).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendIdentity {
    /// UUID de la cuenta de Minecraft (cadena con guiones).
    pub user_id: String,
    /// Nombre de usuario de Minecraft visible.
    pub username: String,
}

impl FriendIdentity {
    pub fn new(user_id: String, username: String) -> Self {
        Self { user_id, username }
    }
}

/// Un amigo guardado en la lista local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    pub identity: FriendIdentity,
    /// Apodo local opcional (editable por el usuario).
    #[serde(default)]
    pub label: Option<String>,
    /// Última IP:puerto conocida (para reconexión rápida).
    #[serde(default)]
    pub last_endpoint: Option<String>,
    /// Última vez que se vio conectado (epoch segundos).
    #[serde(default)]
    pub last_seen: Option<i64>,
    /// Fecha en que se añadió (epoch segundos).
    pub added_at: i64,
}
