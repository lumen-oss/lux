use clap::Args;
use itertools::Itertools as _;
use lux_lib::{config::Config, lockfile::PinnedState, lua_version::LuaVersion, tree::InstallTree};
use miette::{IntoDiagnostic, Result};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

use crate::args::OutputFormat;

#[derive(Args)]
pub struct ListCmd {
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

/// List rocks that are installed in the user tree
pub fn list_installed(list_data: ListCmd, config: Config) -> Result<()> {
    let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;
    let available_rocks = tree.list()?;

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
