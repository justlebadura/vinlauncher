use std::str::FromStr;

use oneclient_db::dao::{bundle as bundle_dao, cluster as cluster_dao};

use crate::LauncherResult;
use crate::state::LauncherState;
use oneclient_common::domain::GameLoader;
use oneclient_common::version::format_mc_version;

use oneclient_cluster::Cluster;
use oneclient_cluster::CreateClusterOptions;

#[tracing::instrument(skip(state))]
pub async fn ensure_from_bundles(state: &LauncherState) -> LauncherResult<Vec<Cluster>> {
    let groups = bundle_dao::list_distinct_version_loaders(&state.services.db).await?;
    let mut created = Vec::new();

    for group in groups {
        let Some(loader) = GameLoader::from_repr(group.mc_loader as u8) else {
            tracing::warn!(
                mc_version = %group.mc_version,
                mc_loader = group.mc_loader,
                "skipping bundle group with unknown loader"
            );
            continue;
        };

        if cluster_dao::find_by_version_loader(
            &state.services.db,
            &group.mc_version,
            group.mc_loader,
        )
        .await?
        .is_some()
        {
            continue;
        }

        let mc_version = group.mc_version.clone();
        let name = format!("{mc_version} {loader}");
        let global = state.settings.read().global_game_settings.clone();
        match state
            .clusters
            .create_provisioned(
                &global,
                CreateClusterOptions::new(name, mc_version.clone(), loader),
            )
            .await
        {
            Ok(Some(cluster)) => {
                tracing::info!(
                    cluster_id = cluster.id,
                    mc_version = %cluster.mc_version,
                    loader = %cluster.mc_loader,
                    "created cluster from bundle catalog"
                );
                created.push(cluster);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    mc_version = %mc_version,
                    loader = %loader,
                    error = %err,
                    "failed to create cluster for bundle group"
                );
            }
        }
    }

    Ok(created)
}

// vinlauncher: ensure_from_versions was removed on purpose - the launcher
// never auto-creates one instance per version of the Polyfrost manifest.
// Instances are created by the user (onboarding or the instance page).
