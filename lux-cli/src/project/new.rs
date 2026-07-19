use std::{error::Error, fmt::Display, path::PathBuf, str::FromStr};

use clap::Args;
use inquire::{
    ui::{RenderConfig, Styled},
    validator::Validation,
    Confirm, Select, Text,
};
use itertools::Itertools;
use miette::{miette, IntoDiagnostic, Result};
use spdx::LicenseId;

use crate::utils::github_metadata::{self, RepoMetadata};
use lux_lib::{
    config::Config,
    package::PackageReq,
    project::{Project, PROJECT_TOML},
};

// TODO:
// - Automatically detect build type to insert into rockspec by inspecting the current repo.
//   E.g. if there is a `Cargo.toml` in the project root we can infer the user wants to use the
//   Rust build backend.

/// The type of directory to create when making the project.
#[derive(Debug, Clone, clap::ValueEnum)]
enum SourceDirType {
    Src,
    Lua,
}

impl Display for SourceDirType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Src => write!(f, "src"),
            Self::Lua => write!(f, "lua"),
        }
    }
}

#[derive(Args)]
pub struct NewProject {
    /// The directory of the project.
    target: PathBuf,

    /// The project's name.
    #[arg(long)]
    name: Option<String>,

    /// The description of the project.
    #[arg(long)]
    description: Option<String>,

    /// The license of the project. Generic license names will be inferred.
    #[arg(long, value_parser = clap_parse_license)]
    license: Option<LicenseId>,

    /// The maintainer of this project. Does not have to be the code author.
    #[arg(long)]
    maintainer: Option<String>,

    /// A comma-separated list of labels to apply to this project.
    #[arg(long, value_parser = clap_parse_list)]
    labels: Option<std::vec::Vec<String>>, // Note: full qualified name required, see https://github.com/clap-rs/clap/issues/4626

    /// A version constraint on the required Lua version for this project.
    /// Examples: ">=5.1", "5.1"
    #[arg(long, value_parser = clap_parse_version)]
    lua_versions: Option<PackageReq>,

    #[arg(long)]
    main: Option<SourceDirType>,
}

struct NewProjectValidated {
    target: PathBuf,
    name: String,
    description: String,
    maintainer: String,
    labels: Vec<String>,
    lua_versions: PackageReq,
    main: SourceDirType,
    license: Option<LicenseId>,
}

fn clap_parse_license(s: &str) -> std::result::Result<LicenseId, String> {
    match validate_license(s) {
        Ok(Validation::Valid) => unsafe { Ok(parse_license_unchecked(s)) },
        Err(_) | Ok(Validation::Invalid(_)) => {
            Err(format!("unable to identify license {s}, please try again!"))
        }
    }
}

fn clap_parse_version(input: &str) -> std::result::Result<PackageReq, String> {
    PackageReq::from_str(format!("lua {input}").as_str()).map_err(|err| err.to_string())
}

fn clap_parse_list(input: &str) -> std::result::Result<Vec<String>, String> {
    if let Some((pos, char)) = input
        .chars()
        .find_position(|&c| c != '-' && c != '_' && c != ',' && c.is_ascii_punctuation())
    {
        Err(format!(
            r#"Unexpected punctuation '{char}' found at column {pos}.
    Lists are comma separated but names should not contain punctuation!"#
        ))
    } else {
        Ok(input.split(',').map(|str| str.trim().to_string()).collect())
    }
}

fn parse_license(input: &str) -> Option<LicenseId> {
    spdx::license_id(input).or_else(|| spdx::imprecise_license_id(input).map(|li| li.0))
}

/// Parses a license.
///
/// # Security
///
/// WARNING: This should only be invoked after validating the license with [`validate_license`].
unsafe fn parse_license_unchecked(input: &str) -> LicenseId {
    parse_license(input).unwrap_unchecked()
}

fn validate_license(input: &str) -> std::result::Result<Validation, Box<dyn Error + Send + Sync>> {
    if input == "none" {
        return Ok(Validation::Valid);
    }

    Ok(
        match parse_license(input).ok_or(format!(
            r#"Unable to identify SPDX license '{input}', please try again!
Supported SPDX license IDs are:
{}
            "#,
            spdx::identifiers::LICENSES
                .iter()
                .map(|license| license.name)
                .join(", ")
        )) {
            Ok(_) => Validation::Valid,
            Err(err) => Validation::Invalid(err.into()),
        },
    )
}

pub async fn write_project_rockspec(cli_flags: NewProject, config: Config) -> Result<()> {
    let project = Project::from_exact(cli_flags.target.clone())?;
    let render_config = RenderConfig::default_colored()
        .with_prompt_prefix(Styled::new(">").with_fg(inquire::ui::Color::LightGreen));

    // If the project already exists then ask for override confirmation
    if project.is_some() && (config.no_prompt() || !overwrite_prompt_confirmed(render_config)?) {
        return Err(miette!("cancelled creation of project (already exists)"));
    };

    let validated = match cli_flags {
        // If all parameters are provided then don't bother prompting the user
        NewProject {
            description: Some(description),
            main: Some(main),
            labels: Some(labels),
            lua_versions: Some(lua_versions),
            maintainer: Some(maintainer),
            name: Some(name),
            license,
            target,
        } => Ok::<_, miette::Report>(NewProjectValidated {
            description,
            labels,
            license,
            lua_versions,
            main,
            maintainer,
            name,
            target,
        }),

        NewProject {
            description,
            labels,
            license,
            lua_versions,
            main,
            maintainer,
            name,
            target,
        } => {
            eprintln!("Fetching remote repository metadata...");
            let repo_metadata = match github_metadata::get_metadata_for(Some(&target)).await {
                Ok(value) => value.map_or_else(|| RepoMetadata::default(&target), Ok),
                Err(_) => {
                    tracing::info!(
                        "Could not fetch remote repo metadata, defaulting to empty values."
                    );

                    RepoMetadata::default(&target)
                }
            }
            .into_diagnostic()?;

            eprintln!("✔ Fetched remote repository metadata.");

            let package_name = name
                .map_or_else(
                    || {
                        Text::new("Package name:")
                            .with_default(&repo_metadata.name)
                            .with_help_message(
                                "A folder with the same name will be created for you.",
                            )
                            .with_render_config(render_config)
                            .prompt()
                    },
                    Ok,
                )
                .into_diagnostic()?;

            let description = description
                .map_or_else(
                    || {
                        Text::new("Description:")
                            .with_default(&repo_metadata.description.unwrap_or_default())
                            .with_render_config(render_config)
                            .prompt()
                    },
                    Ok,
                )
                .into_diagnostic()?;

            let license = license.map_or_else(
                || {
                    Ok::<_, miette::Report>(
                        match Text::new("License:")
                            .with_default(&repo_metadata.license.unwrap_or("none".into()))
                            .with_help_message("Type 'none' for no license")
                            .with_validator(validate_license)
                            .with_render_config(render_config)
                            .prompt()
                            .into_diagnostic()?
                            .as_str()
                        {
                            "none" => None,
                            license => unsafe { Some(parse_license_unchecked(license)) },
                        },
                    )
                },
                |license| Ok(Some(license)),
            )?;

            let labels = labels.or(repo_metadata.labels).map_or_else(
                || {
                    Ok::<_, miette::Report>(
                        Text::new("Labels:")
                            .with_placeholder("web,filesystem")
                            .with_help_message("Labels are comma separated")
                            .prompt()
                            .into_diagnostic()?
                            .split(',')
                            .map(|label| label.trim().to_string())
                            .collect_vec(),
                    )
                },
                Ok,
            )?;

            let maintainer = maintainer
                .map_or_else(
                    || {
                        let prompt = Text::new("Maintainer:");
                        if let Some(default_maintainer) = repo_metadata
                            .contributors
                            .first()
                            .cloned()
                            .or_else(|| whoami::realname().ok())
                        {
                            prompt.with_default(&default_maintainer).prompt()
                        } else {
                            prompt.prompt()
                        }
                    },
                    Ok,
                )
                .into_diagnostic()?;

            let lua_versions = lua_versions.map_or_else(
                || {
                    Ok::<_, miette::Report>(
                        format!(
                            "lua >= {}",
                            Select::new(
                                "What is the lowest Lua version you support?",
                                vec!["5.1", "5.2", "5.3", "5.4", "5.5"]
                            )
                            .without_filtering()
                            .with_vim_mode(true)
                            .with_help_message(
                                "This is equivalent to the 'lua >= {version}' constraint."
                            )
                            .prompt()
                            .into_diagnostic()?
                        )
                        .parse()?,
                    )
                },
                Ok,
            )?;

            Ok(NewProjectValidated {
                target,
                name: package_name,
                description,
                labels,
                license,
                lua_versions,
                maintainer,
                main: main.unwrap_or(SourceDirType::Src),
            })
        }
    }?;

    let _ = std::fs::create_dir_all(&validated.target).into_diagnostic();

    let rocks_path = validated.target.join(PROJECT_TOML);

    std::fs::write(
        &rocks_path,
        format!(
            r#"
package = "{package_name}"
version = "0.1.0"
lua = "{lua_version_req}"

[description]
summary = "{summary}"
maintainer = "{maintainer}"
labels = [ {labels} ]
{license}

[dependencies]
# Add your dependencies here
# `busted = ">=2.0"`

[run]
args = [ "{main}/main.lua" ]

[build]
type = "builtin"
    "#,
            package_name = validated.name,
            summary = validated.description,
            license = validated
                .license
                .map(|license| format!(r#"license = "{}""#, license.name))
                .unwrap_or_default(),
            maintainer = validated.maintainer,
            labels = validated
                .labels
                .into_iter()
                .map(|label| "\"".to_string() + &label + "\"")
                .join(", "),
            lua_version_req = validated.lua_versions.version_req(),
            main = validated.main,
        )
        .trim(),
    )
    .into_diagnostic()?;

    let main_dir = validated.target.join(validated.main.to_string());
    if main_dir.exists() {
        tracing::info!(
            "Directory `{}/` already exists - we won't make any changes to it.",
            main_dir.display()
        );
    } else {
        std::fs::create_dir(&main_dir).into_diagnostic()?;
        std::fs::write(main_dir.join("main.lua"), r#"print("Hello world!")"#).into_diagnostic()?;
    }

    println!("All done!");

    Ok(())
}

fn overwrite_prompt_confirmed<'a>(render_config: RenderConfig<'a>) -> Result<bool> {
    Confirm::new("Target directory already has a project, write anyway?")
        .with_default(false)
        .with_help_message(&format!("This may overwrite your existing {PROJECT_TOML}",))
        .with_render_config(render_config)
        .prompt()
        .into_diagnostic()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_license() {
        // non-regression for #1612
        assert!(parse_license("EUPL-1.2").is_some());
        assert!(parse_license("agplv3").is_some());
    }
}

// TODO(vhyrro): Add more tests
