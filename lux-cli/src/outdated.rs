use std::collections::HashMap;

use clap::Args;
use itertools::Itertools;
use lux_lib::{
    config::Config, lockfile::LockedPackage, lua_version::LuaVersion, package::PackageVersion,
    package_db::PackageDB, workspace::Workspace,
};

use miette::{IntoDiagnostic, Result};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

use crate::args::OutputFormat;

#[derive(Args)]
pub struct Outdated {
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

/// List rocks that are outdated
/// If in a project, this lists rocks in the project tree
pub async fn outdated(outdated_data: Outdated, config: Config) -> Result<()> {
    let workspace = Workspace::current()?;
    let tree = match &workspace {
        Some(project) => project.tree(&config)?,
        None => {
            let lua_version = LuaVersion::from(&config)?.clone();
            config.user_tree(lua_version)?
        }
    };

    let package_db = PackageDB::from_config(&config).await?;

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
                .ok()
                .flatten()
                .map(|version| (rock, version))
        })
        .collect::<Vec<(&LockedPackage, PackageVersion)>>();

    let rock_list = rock_list
        .iter()
        .sorted_by_key(|(rock, _)| rock.name().to_owned())
        .into_group_map_by(|(rock, _)| rock.name().to_owned());

    match outdated_data.output_format {
        OutputFormat::Json => {
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

            println!(
                "{}",
                serde_json::to_string(&jsonified_rock_list).into_diagnostic()?
            );
        }
        OutputFormat::Text => {
            let formatting = TreeFormatting::dir_tree(FormatCharacters::box_chars());

            for (rock_name, updates) in rock_list {
                let mut tree = StringTreeNode::new(rock_name.to_string());

                for (rock, latest_version) in updates {
                    tree.push(format!("{} => {}", rock.version(), latest_version));
                }

                println!(
                    "{}",
                    tree.to_string_with_format(&formatting).into_diagnostic()?
                );
            }
        }
    }

    println!("\nRun `lx update` to update all outdated rocks.");

    Ok(())
}
