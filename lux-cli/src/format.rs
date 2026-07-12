use std::path::{Path, PathBuf};

use clap::Args;
use emmylua_formatter as luafmt;
use eyre::{bail, Context, Result};
use lux_lib::{
    config::Config, lua_version::LuaVersion, package::PackageName, project::Project,
    workspace::Workspace,
};
use path_slash::PathExt;
use walkdir::WalkDir;

use crate::utils::path::{classify_path, PathTarget};

#[derive(Args)]
pub struct Fmt {
    /// Path to a workspace, directory, or Lua file to format. Defaults to the current workspace.
    #[arg(long)]
    path: Option<PathBuf>,

    #[clap(default_value = "stylua")]
    #[arg(long)]
    backend: FmtBackend,

    /// Package to format.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
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
    let target = match args.path.as_deref() {
        None => PathTarget::Workspace(Box::new(Workspace::current_or_err()?)),
        Some(path) => classify_path(path)?,
    };
    match target {
        PathTarget::Workspace(workspace) => {
            if let Some(package) = &args.package {
                let project = workspace.select_member(package)?;
                format_project(&args, &workspace, project, &config)?;
            } else {
                for project in workspace.members() {
                    format_project(&args, &workspace, project, &config)?;
                }
            }
        }
        PathTarget::File(file) => {
            ensure_no_package(&args)?;
            let root = file
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            format_loose(std::iter::once(file), &root, &args.backend, &config)?;
        }
        PathTarget::Directory(dir) => {
            ensure_no_package(&args)?;
            let files = WalkDir::new(&dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.into_path())
                .filter(|path| is_lua_source(path));
            format_loose(files, &dir, &args.backend, &config)?;
        }
    }
    Ok(())
}

struct FmtConfig {
    stylua: stylua_lib::Config,
    luafmt: luafmt::LuaFormatConfig,
    luafmt_syntax_level: luafmt::LuaSyntaxLevel,
    editorconfig: PathBuf,
}

impl FmtConfig {
    fn resolve(root: &Path, lua_version: Option<LuaVersion>) -> Self {
        let stylua: stylua_lib::Config = std::fs::read_to_string(root.join("stylua.toml"))
            .or_else(|_| std::fs::read_to_string(root.join(".stylua.toml")))
            .map(|config: String| toml::from_str(&config).unwrap_or_default())
            .or_else(|_| {
                stylua_lib::editorconfig::parse(stylua_lib::Config::new(), &root.join("*.lua"))
            })
            .unwrap_or_default();

        let luafmt = luafmt::resolve_config_for_path(Some(root), None)
            .map(|resolved| resolved.config)
            .unwrap_or_default();
        let luafmt_syntax_level = lua_version
            .map(lua_version_to_luafmt_syntax_level)
            .unwrap_or(luafmt.syntax.level);

        Self {
            stylua,
            luafmt,
            luafmt_syntax_level,
            editorconfig: root.join(".editorconfig"),
        }
    }

    fn format(&self, backend: &FmtBackend, path: &Path, code: &str) -> Result<String> {
        Ok(match backend {
            FmtBackend::Stylua => stylua_lib::format_code(
                code,
                self.stylua,
                None,
                stylua_lib::OutputVerification::Full,
            )
            .context(format!("error formatting {} with stylua.", path.display()))?,
            FmtBackend::Luafmt => {
                luafmt::check_text(code, self.luafmt_syntax_level.into(), &self.luafmt).formatted
            }
            FmtBackend::EmmyluaCodestyle => {
                let uri = path.to_slash_lossy().to_string();
                if self.editorconfig.is_file() {
                    emmylua_codestyle::update_code_style(&uri, &self.editorconfig.to_slash_lossy());
                }
                emmylua_codestyle::reformat_code(
                    code,
                    &uri,
                    emmylua_codestyle::FormattingOptions::default(),
                )
            }
        })
    }
}

fn format_files(
    files: impl Iterator<Item = PathBuf>,
    configs: &FmtConfig,
    backend: &FmtBackend,
) -> Result<()> {
    files.into_iter().try_for_each(|file| {
        let unformatted_code = std::fs::read_to_string(&file)?;
        let formatted_code = configs.format(backend, &file, &unformatted_code)?;
        std::fs::write(&file, formatted_code)
            .context(format!("error writing formatted file {}.", file.display()))
    })
}

fn format_project(
    args: &Fmt,
    workspace: &Workspace,
    project: &Project,
    config: &Config,
) -> Result<()> {
    let configs = FmtConfig::resolve(
        workspace.root().as_ref(),
        workspace.lua_version(config).ok(),
    );

    let lua_files = ["src", "lua", "lib", "spec", "test", "tests"]
        .iter()
        .flat_map(|dir| WalkDir::new(project.root().join(dir)))
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|path| is_lua_source(path));

    let rockspec = project.root().join("extra.rockspec");

    format_files(
        lua_files.chain(rockspec.exists().then_some(rockspec)),
        &configs,
        &args.backend,
    )
}

fn is_lua_source(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "lua" || ext == "rockspec")
}

fn ensure_no_package(args: &Fmt) -> Result<()> {
    if args.package.is_some() {
        bail!("--package is only valid within a workspace");
    }
    Ok(())
}

fn format_loose(
    files: impl Iterator<Item = PathBuf>,
    root: &Path,
    backend: &FmtBackend,
    config: &Config,
) -> Result<()> {
    let (config_root, lua_version) = match Workspace::from(root)? {
        Some(workspace) => (
            workspace.root().as_ref().to_path_buf(),
            workspace.lua_version(config).ok(),
        ),
        None => (root.to_path_buf(), config.lua_version().cloned()),
    };
    let configs = FmtConfig::resolve(&config_root, lua_version);
    format_files(files, &configs, backend)
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

#[cfg(test)]
mod tests {
    use assert_fs::fixture::PathChild;
    use assert_fs::{prelude::PathCopy, TempDir};
    use lux_lib::config::ConfigBuilder;
    use serial_test::serial;

    use super::*;
    use std::path::PathBuf;

    #[serial]
    #[tokio::test]
    async fn test_format_while_in_another_workspace() {
        let unformatted_sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/unformatted/");
        let unformatted_project_root = TempDir::new().unwrap();
        unformatted_project_root
            .copy_from(&unformatted_sample_project, &["**"])
            .unwrap();

        let cwd_sample_project =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
        let cwd_project_root = TempDir::new().unwrap();
        cwd_project_root
            .copy_from(&cwd_sample_project, &["**"])
            .unwrap();

        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&cwd_project_root).unwrap();

        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let fmt = Fmt {
            path: Some(unformatted_project_root.to_path_buf()),
            backend: FmtBackend::Stylua,
            package: None,
        };

        format(fmt, config).unwrap();

        let unformatted_file_path = unformatted_project_root.child("src").child("main.lua");
        let content = std::fs::read_to_string(&unformatted_file_path).unwrap();

        // the unformatted variant contains too many spaces
        assert!(content.contains("print(1 * 2)"));

        std::env::set_current_dir(&cwd).unwrap();
    }

    fn loose_lua_temp_dir() -> TempDir {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/loose-lua/");
        let dir = TempDir::new().unwrap();
        dir.copy_from(&fixture, &["**"]).unwrap();
        dir
    }

    fn fmt(path: Option<PathBuf>) -> Fmt {
        Fmt {
            path,
            backend: FmtBackend::Stylua,
            package: None,
        }
    }

    #[test]
    fn test_format_plain_directory_without_lux_toml() {
        let dir = loose_lua_temp_dir();
        let config = ConfigBuilder::new().unwrap().build().unwrap();

        format(fmt(Some(dir.to_path_buf())), config).unwrap();

        let top = std::fs::read_to_string(dir.child("a.lua")).unwrap();
        let nested = std::fs::read_to_string(dir.child("nested").child("b.lua")).unwrap();
        let other = std::fs::read_to_string(dir.child("notes.txt")).unwrap();
        assert!(top.contains("print(1 * 2)"));
        assert!(nested.contains("print(3 + 4)"));
        // non-Lua files are left untouched
        assert!(other.contains("print( 5 *    6 )"));
    }

    #[test]
    fn test_format_single_lua_file() {
        let dir = loose_lua_temp_dir();
        let config = ConfigBuilder::new().unwrap().build().unwrap();

        format(fmt(Some(dir.child("a.lua").to_path_buf())), config).unwrap();

        let top = std::fs::read_to_string(dir.child("a.lua")).unwrap();
        let nested = std::fs::read_to_string(dir.child("nested").child("b.lua")).unwrap();
        assert!(top.contains("print(1 * 2)"));
        // a sibling file is not touched when a single file is targeted
        assert!(nested.contains("print( 3 +    4 )"));
    }

    #[test]
    fn test_format_nonexistent_path_errors() {
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let result = format(fmt(Some("/no/such/path".into())), config);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_subdir_inherits_workspace_config() {
        // must resolve workspace's stylua.toml (Spaces/2-width), not stylua default.
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/stylua-config/");
        let workspace = TempDir::new().unwrap();
        workspace.copy_from(&fixture, &["**"]).unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();

        format(fmt(Some(workspace.child("src").to_path_buf())), config).unwrap();

        let content = std::fs::read_to_string(workspace.child("src").child("main.lua")).unwrap();
        assert!(content.contains("\n  print(1 * 2)"));
        assert!(!content.contains('\t'));
    }
}
