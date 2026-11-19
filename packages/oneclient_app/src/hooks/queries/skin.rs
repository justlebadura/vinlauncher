use std::path::PathBuf;

use freya::prelude::*;
use freya::query::{Query, QueryCapability, UseQuery, use_query};
use oneclient_auth::AccountKind;
use oneclient_core::LauncherError;

use crate::AppAssets;

pub fn use_player_skin(uuid: String) -> (Bytes, bool) {
    let profile = super::use_player_profile(uuid.clone(), None::<String>);
    let accounts_query = super::use_accounts();

    let (skin_url, is_slim) = crate::hooks::settled_or_loading(&profile)
        .map_or((None, false), |profile| (profile.skin_url, profile.is_slim));

    // Offline accounts have no Mojang skin_url: the locally saved skin file
    // under config_dir()/skins/{username}.png stands in for them and only
    // the launcher shows it
    let offline_skin_path = super::try_accounts(&accounts_query)
        .and_then(|accounts| {
            accounts
                .iter()
                .find(|account| account.id.to_string() == uuid)
                .cloned()
        })
        .filter(|account| account.kind == AccountKind::Offline && skin_url.is_none())
        .and_then(|account| offline_skin_path(&account.username));

    let skin_query = super::use_cached_image(skin_url.clone(), 256);
    let offline_query = use_offline_skin(offline_skin_path.clone());

    let steve = use_memo(|| AppAssets::get_bytes("steve.png").unwrap_or_default());
    let alex = use_memo(|| AppAssets::get_bytes("alex.png").unwrap_or_default());

    let default_slim = (java_string_hash(&uuid) & 1) == 1;

    match crate::hooks::loaded_image(skin_url.as_deref(), &skin_query) {
        Some((_, bytes)) => (bytes, is_slim),
        None => match super::settled_or_loading(&offline_query) {
            Some(bytes) => (bytes, default_slim),
            None if default_slim => (alex.read().clone(), true),
            None => (steve.read().clone(), false),
        },
    }
}

fn offline_skin_path(username: &str) -> Option<PathBuf> {
    Some(
        oneclient_common::paths::skins_dir()
            .ok()?
            .join(format!("{username}.png")),
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalSkinKeys {
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalSkinQuery;

impl QueryCapability for LocalSkinQuery {
    type Ok = Bytes;
    type Err = LauncherError;
    type Keys = LocalSkinKeys;

    async fn run(&self, keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let path = keys.path.clone();
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(&path))
            .await
            .map_err(|err| LauncherError::Minecraft(err.to_string()))?
            .map_err(|err| LauncherError::Minecraft(format!("offline skin: {err}")))?;
        Ok(Bytes::from(bytes))
    }
}

pub fn use_offline_skin(path: Option<PathBuf>) -> UseQuery<LocalSkinQuery> {
    let path = path.unwrap_or_default();
    let keys = (!path.as_os_str().is_empty()).then_some(LocalSkinKeys { path });

    use_query(Query::new(keys, LocalSkinQuery))
}

fn java_string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}
