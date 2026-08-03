use inquire::Confirm;
use lux_lib::{
    config::{Config, ConfigBuilder},
    workspace::Workspace,
};
use miette::{miette, Context, IntoDiagnostic, Result};

#[derive(clap::Subcommand)]
pub enum ConfigCmd {
    /// Initialise a new config file
    Init(Init),
    /// Edit the current config file.
    Edit {
        /// Edit the current workspace config.
        #[arg(long)]
        workspace: bool,
    },
    /// Show the current config.
    /// This includes options picked up from CLI flags.
    Show,
}

#[derive(clap::Args)]
pub struct Init {
    /// Initialise the default config for this system.
    /// If this flag is not set, an empty config file will be created.
    #[arg(long, conflicts_with = "current")]
    default: bool,

    /// Initialise the config file using the current config,
    /// with options picked up from CLI flags.
    #[arg(long, conflicts_with = "default")]
    current: bool,

    /// Initialise the config in the current workspace.
    #[arg(long)]
    workspace: bool,
}

pub fn config(cmd: ConfigCmd, config: Config) -> Result<()> {
    match cmd {
        ConfigCmd::Init(init) => {
            let config_file = if init.workspace {
                Workspace::current_or_err().into_diagnostic()?.config_file()
            } else {
                ConfigBuilder::config_file()?
            };
            if !config_file.is_file() && !config.no_prompt()
                || Confirm::new("Config already exists. Overwrite?")
                    .with_default(false)
                    .prompt()
                    .into_diagnostic()
                    .wrap_err("error prompting to overwrite config")?
            {
                std::fs::create_dir_all(
                    config_file
                        .parent()
                        .ok_or_else(|| miette!("error getting lux config parent directory"))?,
                )
                .into_diagnostic()?;
                let content = if init.default {
                    let cfg: ConfigBuilder = ConfigBuilder::default().build()?.into();
                    toml::to_string(&cfg).into_diagnostic()?
                } else if init.current {
                    let cfg: ConfigBuilder = config.into();
                    toml::to_string(&cfg).into_diagnostic()?
                } else {
                    String::default()
                };
                std::fs::write(&config_file, content).into_diagnostic()?;
                print!("Config initialised at {}", config_file.display());
            }
        }
        ConfigCmd::Edit { workspace } => {
            let config_file = if workspace {
                Workspace::current_or_err().into_diagnostic()?.config_file()
            } else {
                ConfigBuilder::config_file()?
            };
            if !config_file.is_file() {
                let workspace_flag = if workspace { " --workspace " } else { "" };
                return Err(miette!(
                    help = format!(
                        r#"
Use 'lx config init{workspace_flag}', 'lx config init{workspace_flag} --default',
or 'lx config init{workspace_flag} --current' to initialise a config file.
"#
                    ),
                    "No config file found."
                ));
            }
            edit::edit_file(config_file).into_diagnostic()?;
        }
        ConfigCmd::Show => {
            let cfg: ConfigBuilder = config.into();
            print!("{}", toml::to_string(&cfg).into_diagnostic()?);
        }
    }
    Ok(())
}
