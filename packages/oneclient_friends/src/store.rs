//! Persistencia de la lista de amigos en un archivo JSON local.

use std::path::{Path, PathBuf};

use tokio::sync::RwLock;

use crate::identity::{FriendEntry, FriendIdentity};

/// Lista de amigos persistida.
#[derive(Debug, Default)]
pub struct FriendsStore {
    path: PathBuf,
    friends: RwLock<Vec<FriendEntry>>,
}

impl FriendsStore {
    /// Carga (o crea) la lista desde `path`.
    pub async fn load(path: PathBuf) -> std::io::Result<Self> {
        let friends = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path,
            friends: RwLock::new(friends),
        })
    }

    pub async fn list(&self) -> Vec<FriendEntry> {
        self.friends.read().await.clone()
    }

    pub async fn add(&self, identity: FriendIdentity) -> std::io::Result<bool> {
        let mut guard = self.friends.write().await;
        if guard.iter().any(|f| f.identity.user_id == identity.user_id) {
            return Ok(false); // ya estaba
        }
        guard.push(FriendEntry {
            identity,
            label: None,
            last_endpoint: None,
            last_seen: None,
            added_at: chrono_now(),
        });
        Self::persist_locked(&self.path, guard.as_slice()).await?;
        Ok(true)
    }

    pub async fn remove(&self, user_id: &str) -> std::io::Result<bool> {
        let mut guard = self.friends.write().await;
        let before = guard.len();
        guard.retain(|f| f.identity.user_id != user_id);
        if guard.len() == before {
            return Ok(false);
        }
        Self::persist_locked(&self.path, guard.as_slice()).await?;
        Ok(true)
    }

    /// Actualiza el último endpoint/avistamiento conocido de un amigo.
    pub async fn touch(&self, user_id: &str, endpoint: Option<String>) -> std::io::Result<()> {
        let mut guard = self.friends.write().await;
        for f in guard.iter_mut() {
            if f.identity.user_id == user_id {
                f.last_seen = Some(chrono_now());
                if endpoint.is_some() {
                    f.last_endpoint = endpoint.clone();
                }
            }
        }
        Self::persist_locked(&self.path, guard.as_slice()).await
    }

    async fn persist_locked(path: &Path, friends: &[FriendEntry]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(friends).expect("friends serialize");
        // Escritura atómica: tmp + rename.
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
