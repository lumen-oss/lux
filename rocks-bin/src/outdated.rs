use std::collections::HashMap;

use clap::Args;
use eyre::Result;
use itertools::Itertools;
use rocks_lib::{
    config::{Config, LuaVersion},
    progress::{MultiProgress, Progress},
    remote_package_db::RemotePackageDB,
    tree::Tree,
};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

#[derive(Args)]
pub struct Outdated {
    #[arg(long)]
    porcelain: bool,
}

pub async fn outdated(outdated_data: Outdated, config: Config) -> Result<()> {
    let progress = MultiProgress::new();
    let bar = Progress::Progress(progress.new_bar());
    let tree = Tree::new(config.tree().clone(), LuaVersion::from(&config)?)?;

    let package_db = RemotePackageDB::from_config(&config, &bar).await?;

    bar.map(|b| b.set_message("🔎 Checking for outdated rocks...".to_string()));

    // NOTE: This will display all installed versions and each possible upgrade.
    // However, this should also take into account dependency constraints made by other rocks.
    // This will naturally occur with lockfiles and should be accounted for directly in the
    // `has_update` function.
    let rock_list = tree.as_rock_list()?;
    let rock_list = rock_list
        .iter()
        .filter_map(|rock| {
            rock.to_package()
                .has_update(&package_db)
                .expect("TODO")
                .map(|version| (rock, version))
        })
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
