use eyre::{eyre, Context, OptionExt, Result};
use inquire::Confirm;
use lux_lib::config::{Config, ConfigBuilder};

#[derive(clap::Subcommand)]
pub enum ConfigCmd {
    /// Initialise a new config file
    Init(Init),
    /// Edit the current config file.
    Edit,
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
}

pub fn config(cmd: ConfigCmd, config: Config) -> Result<()> {
    match cmd {
        ConfigCmd::Init(init) => {
            let config_file = ConfigBuilder::config_file()?;
            if !config_file.is_file()
                || Confirm::new("Config already exists. Overwrite?")
                    .with_default(false)
                    .prompt()
                    .wrap_err("error prompting to overwrite config")?
            {
                std::fs::create_dir_all(
                    config_file
                        .parent()
                        .ok_or_eyre("error getting lux config parent directory")?,
                )?;
                let content = if init.default {
                    let cfg: ConfigBuilder = ConfigBuilder::default().build()?.into();
                    toml::to_string(&cfg)?
                } else if init.current {
                    let cfg: ConfigBuilder = config.into();
                    toml::to_string(&cfg)?
                } else {
                    String::default()
                };
                std::fs::write(&config_file, content)?;
                print!("Config initialised at {}", config_file.display());
            }
        }
        ConfigCmd::Edit => {
            let config_file = ConfigBuilder::config_file()?;
            if !config_file.is_file() {
                return Err(eyre!(
                    "
No config file found.
Use 'lux config init', 'lux config init --default', or 'lux config init --current'
to initialise a config file.
"
                ));
            }
            edit::edit_file(config_file)?;
        }
        ConfigCmd::Show => {
            let cfg: ConfigBuilder = config.into();
            print!("{}", toml::to_string(&cfg)?);
        }
    }
    Ok(())
}
