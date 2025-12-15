use std::collections::HashMap;

use clap::Args;
use eyre::Result;
use itertools::Itertools;
use lux_lib::{
    config::{Config, LuaVersion},
    progress::MultiProgress,
    project::Project,
    remote_package_db::PackageDB,
};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

use crate::utils::project::sync_dependencies_if_locked;

#[derive(Args)]
pub struct Outdated {
    #[arg(long)]
    porcelain: bool,
}

/// List rocks that are outdated
/// If in a project, this lists rocks in the project tree
pub async fn outdated(outdated_data: Outdated, config: Config) -> Result<()> {
    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let project = Project::current()?;
    let tree = match &project {
        Some(project) => {
            // Make sure dependencies are synced if in a project
            sync_dependencies_if_locked(project, MultiProgress::new_arc(&config), &config).await?;
            project.tree(&config)?
        }
        None => {
            let lua_version = LuaVersion::from(&config)?.clone();
            config.user_tree(lua_version)?
        }
    };

    let package_db = PackageDB::from_config(&config, &bar).await?;

    bar.map(|b| b.set_message("🔎 Checking for outdated rocks...".to_string()));

    // NOTE: This will display all installed versions and each possible upgrade.
    // However, this should also take into account dependency constraints made by other rocks.
    // This will naturally occur with lockfiles and should be accounted for directly in the
    // `has_update` function.
    let rock_list = tree.as_rock_list()?;
    let mut updates = Vec::new();
    for rock in &rock_list {
        if let Some(version) = rock.to_package().has_update(&package_db).await? {
            updates.push((rock, version));
        }
    }
    let rock_list = updates;

    let rock_list = rock_list
        .iter()
        .sorted_by_key(|(rock, _)| rock.name().to_owned())
        .into_group_map_by(|(rock, _)| rock.name().to_owned());

    bar.map(|b| b.finish_and_clear());

    if outdated_data.porcelain {
        let jsonified_rock_list = rock_list
            .iter()
            .map(|(key, values)| {
                (
                    key,
                    values
                        .iter()
                        .map(|(k, v)| (k.version().to_string(), v.to_string()))
                        .collect::<HashMap<_, _>>(),
                )
            })
            .collect::<HashMap<_, _>>();

        println!("{}", serde_json::to_string(&jsonified_rock_list)?);
    } else {
        let formatting = TreeFormatting::dir_tree(FormatCharacters::box_chars());

        for (rock_name, updates) in rock_list {
            let mut tree = StringTreeNode::new(rock_name.to_string());

            for (rock, latest_version) in updates {
                tree.push(format!("{} => {}", rock.version(), latest_version));
            }

            println!("{}", tree.to_string_with_format(&formatting)?);
        }
    }

    Ok(())
}
