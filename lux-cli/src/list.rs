use std::collections::HashSet;

use clap::Args;
use itertools::Itertools as _;
use lux_lib::{
    config::Config,
    lockfile::{LocalPackageId, PinnedState},
    lua_version::LuaVersion,
    tree::InstallTree,
};
use miette::{IntoDiagnostic, Result};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

use crate::args::OutputFormat;

#[derive(Args)]
pub struct ListCmd {
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,

    /// Only list rocks that are not reachable from an entrypoint.
    #[arg(long)]
    orphans: bool,

    /// Only list rocks that are not a dependency of any other rock.
    #[arg(long)]
    removable: bool,
}

/// List rocks that are installed in the user tree
pub fn list_installed(list_data: ListCmd, config: Config) -> Result<()> {
    let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;
    let lockfile = tree.lockfile()?;
    let mut available_rocks = tree.list()?;
    if list_data.orphans {
        let reachable: HashSet<LocalPackageId> = lockfile
            .reachable_rocks()
            .into_iter()
            .map(|package| package.id())
            .collect();
        available_rocks.retain(|_, packages| {
            packages.retain(|package| !reachable.contains(&package.id()));
            !packages.is_empty()
        });
    }
    if list_data.removable {
        available_rocks.retain(|_, packages| {
            packages.retain(|package| !lockfile.is_dependency(&package.id()));
            !packages.is_empty()
        });
    }

    match list_data.output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string(&available_rocks).into_diagnostic()?
            );
        }
        OutputFormat::Text => {
            let formatting = TreeFormatting::dir_tree(FormatCharacters::box_chars());
            for (name, packages) in available_rocks.into_iter().sorted() {
                let mut tree = StringTreeNode::new(name.to_string());

                for package in packages {
                    tree.push(format!(
                        "{}{}",
                        package.version(),
                        if package.pinned() == PinnedState::Pinned {
                            " (pinned)"
                        } else {
                            ""
                        }
                    ));
                }

                println!(
                    "{}",
                    tree.to_string_with_format(&formatting).into_diagnostic()?
                );
            }
        }
    }

    Ok(())
}
