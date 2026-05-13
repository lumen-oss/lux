use std::path::PathBuf;

use clap::Args;
use emmylua_formatter as luafmt;
use eyre::{Context, OptionExt, Result};
use lux_lib::{config::Config, lua_version::LuaVersion, project::Project};
use path_slash::PathExt;
use walkdir::WalkDir;

#[derive(Args)]
pub struct Fmt {
    /// Optional path to a workspace or Lua file to format.
    workspace_or_file: Option<PathBuf>,

    #[clap(default_value = "stylua")]
    #[arg(long)]
    backend: FmtBackend,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum FmtBackend {
    /// Mainly follows the [Roblox Lua style guide](https://roblox.github.io/lua-style-guide/).
    Stylua,
    /// The default formatter used by [emmylua-analyzer-rust](https://github.com/EmmyLuaLs/emmylua-analyzer-rust).
    /// If invoked with `lx --lua-version=<version> fmt`, Lux will configure the luafmt syntax level
    /// to match the specified Lua version.
    Luafmt,
    /// The default formatter used by [lua-language-server](https://luals.github.io/).
    EmmyluaCodestyle,
}

pub fn format(args: Fmt, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre(
        "`lx fmt` can only be executed in a lux project! Run `lx new` to create one.",
    )?;

    let stylua_config: stylua_lib::Config = std::fs::read_to_string("stylua.toml")
        .or_else(|_| std::fs::read_to_string(".stylua.toml"))
        .map(|config: String| toml::from_str(&config).unwrap_or_default())
        .or_else(|_| {
            stylua_lib::editorconfig::parse(
                stylua_lib::Config::new(),
                &project.root().join("*.lua"),
            )
        })
        .unwrap_or_default();

    let luafmt_config = luafmt::resolve_config_for_path(Some(project.root().as_ref()), None)
        .map(|resolved| resolved.config)
        .unwrap_or_default();
    let luafmt_syntax_level = project
        .lua_version(&config)
        .map(lua_version_to_luafmt_syntax_level)
        .unwrap_or(luafmt_config.syntax.level);

    let emmylua_config = project.root().join(".editorconfig");

    let workspace_or_file = args
        .workspace_or_file
        .map(std::path::absolute)
        .transpose()?;

    WalkDir::new(project.root().join("src"))
        .into_iter()
        .chain(WalkDir::new(project.root().join("lua")))
        .chain(WalkDir::new(project.root().join("lib")))
        .chain(WalkDir::new(project.root().join("spec")))
        .chain(WalkDir::new(project.root().join("test")))
        .chain(WalkDir::new(project.root().join("tests")))
        .filter_map(Result::ok)
        .filter(|file| {
            workspace_or_file
                .as_ref()
                .is_none_or(|workspace_or_file| file.path().starts_with(workspace_or_file))
        })
        .try_for_each(|file| {
            if PathBuf::from(file.file_name())
                .extension()
                .is_some_and(|ext| ext == "lua")
            {
                let file = file.path();
                let unformatted_code = std::fs::read_to_string(file)?;
                let formatted_code = match args.backend {
                    FmtBackend::Stylua => stylua_lib::format_code(
                        &unformatted_code,
                        stylua_config,
                        None,
                        stylua_lib::OutputVerification::Full,
                    )
                    .context(format!("error formatting {} with stylua.", file.display()))?,
                    FmtBackend::Luafmt => {
                        luafmt::check_text(
                            &unformatted_code,
                            luafmt_syntax_level.into(),
                            &luafmt_config,
                        )
                        .formatted
                    }
                    FmtBackend::EmmyluaCodestyle => {
                        let uri = file.to_slash_lossy().to_string();
                        if emmylua_config.is_file() {
                            emmylua_codestyle::update_code_style(
                                &uri,
                                &emmylua_config.to_slash_lossy(),
                            );
                        }
                        emmylua_codestyle::reformat_code(
                            &unformatted_code,
                            &uri,
                            emmylua_codestyle::FormattingOptions::default(),
                        )
                    }
                };

                std::fs::write(file, formatted_code)
                    .context(format!("error writing formatted file {}.", file.display()))?
            };
            Ok::<_, eyre::Report>(())
        })?;

    // Format the rockspec

    let rockspec = project.root().join("extra.rockspec");

    if rockspec.exists() {
        let unformatted_code = std::fs::read_to_string(&rockspec)?;
        let formatted_code = match args.backend {
            FmtBackend::Stylua => stylua_lib::format_code(
                &unformatted_code,
                stylua_config,
                None,
                stylua_lib::OutputVerification::Full,
            )?,
            FmtBackend::Luafmt => {
                luafmt::check_text(
                    &unformatted_code,
                    luafmt_syntax_level.into(),
                    &luafmt_config,
                )
                .formatted
            }
            FmtBackend::EmmyluaCodestyle => {
                let uri = rockspec.to_slash_lossy().to_string();
                if emmylua_config.is_file() {
                    emmylua_codestyle::update_code_style(&uri, &emmylua_config.to_slash_lossy());
                }
                emmylua_codestyle::reformat_code(
                    &unformatted_code,
                    &uri,
                    emmylua_codestyle::FormattingOptions::default(),
                )
            }
        };

        std::fs::write(rockspec, formatted_code)?;
    }

    Ok(())
}

fn lua_version_to_luafmt_syntax_level(lua_version: LuaVersion) -> luafmt::LuaSyntaxLevel {
    match lua_version {
        LuaVersion::Lua51 => luafmt::LuaSyntaxLevel::Lua51,
        LuaVersion::Lua52 => luafmt::LuaSyntaxLevel::Lua52,
        LuaVersion::Lua53 => luafmt::LuaSyntaxLevel::Lua53,
        LuaVersion::Lua54 => luafmt::LuaSyntaxLevel::Lua54,
        LuaVersion::Lua55 => luafmt::LuaSyntaxLevel::Lua55,
        LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => luafmt::LuaSyntaxLevel::LuaJIT,
    }
}
