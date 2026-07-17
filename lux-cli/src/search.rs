use std::collections::HashMap;

use clap::Args;
use itertools::Itertools;
use miette::{IntoDiagnostic, Result};
use text_trees::{FormatCharacters, StringTreeNode, TreeFormatting};

use lux_lib::{
    config::Config,
    package::{PackageName, PackageReq, PackageVersion},
    remote_package_db::RemotePackageDB,
};

use crate::args::OutputFormat;

#[derive(Args)]
pub struct Search {
    lua_package_req: PackageReq,
    // TODO(vhyrro): Add options.
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

pub async fn search(data: Search, config: Config) -> Result<()> {
    let formatting = TreeFormatting::dir_tree(FormatCharacters::box_chars());

    let package_db = RemotePackageDB::from_config(&config).await?;

    let lua_package_req = data.lua_package_req;

    let result = package_db.search(&lua_package_req);

    match data.output_format {
        OutputFormat::Json => {
            let rock_to_version_map: HashMap<&PackageName, Vec<&PackageVersion>> =
                HashMap::from_iter(result);
            println!(
                "{}",
                serde_json::to_string(&rock_to_version_map).into_diagnostic()?
            );
        }
        OutputFormat::Text => {
            for (key, versions) in result.into_iter().sorted() {
                let mut tree = StringTreeNode::new(key.to_string().to_owned());

                for version in versions {
                    tree.push(version.to_string());
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
