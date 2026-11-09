use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

use miette::Diagnostic;
use path_slash::PathExt as _;
use thiserror::Error;

use crate::{
    config::Config,
    fs, lua,
    path::{self, Paths},
    tree::{self, InstallTree},
};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Paths(#[from] path::Error),
    #[error("the Teal compiler was not found in the build tree")]
    #[diagnostic(help("add `tl` to `build_dependencies` and rerun the build"))]
    NoTealCompiler,
    #[error("failed to run the Teal compiler")]
    #[diagnostic(help("ensure Lua is installed and executable, or run `lx install-lua`"))]
    Run { source: io::Error },
    #[error("no Lua executable found")]
    NoLuaBinary {
        #[help]
        help: String,
    },
    #[error("failed to compile Teal module '{}':\n{stderr}", file.display())]
    #[diagnostic(help(
        "fix the Teal errors above, or verify `tl` supports the target Lua version"
    ))]
    Compile { file: PathBuf, stderr: String },
}

pub(crate) async fn compile<T: InstallTree>(
    build_dir: &Path,
    target_dir: &Path,
    files: &[PathBuf],
    lua: &lua::Installation,
    tree: &T,
    config: &Config,
) -> Result<(), Error> {
    let build_dir = fs::sync::absolute(build_dir)?;
    let target_dir = fs::sync::absolute(target_dir)?;
    let build_tree = tree.build_tree(config)?;
    let script = teal_script(&build_tree).ok_or(Error::NoTealCompiler)?;
    let build_paths = Paths::new(&build_tree)?;
    let target_paths = Paths::new(tree)?;
    let lua_path = lua_path(&build_dir, &build_paths, &target_paths);
    let lua_cpath = [
        build_paths.package_cpath().joined(),
        target_paths.package_cpath().joined(),
    ]
    .into_iter()
    .filter(|path| !path.is_empty())
    .collect::<Vec<_>>()
    .join(";");
    let lua_bin = lua
        .lua_binary_or_config_override(config)
        .ok_or_else(|| Error::NoLuaBinary {
            help: format!(
                "make sure Lua {} is on the PATH or has been installed by Lux",
                lua.version
            ),
        })?;
    let version_suffix = lua.version.version_compatibility_str().replace('.', "_");
    let (gen_target, gen_compat) = gen_options(&lua.version);
    let compiler = Compiler {
        script: &script,
        working_dir: &build_dir,
        lua_bin: &lua_bin,
        config,
        lua_path: &lua_path,
        lua_cpath: &lua_cpath,
        version_suffix: &version_suffix,
        gen_target,
        gen_compat,
    };
    for file in files.iter().filter(|file| !is_declaration(file)) {
        let target = target_dir
            .join(file.components().skip(1).collect::<PathBuf>())
            .with_extension("lua");
        compiler.compile(&build_dir.join(file), &target).await?;
    }
    Ok(())
}

struct Compiler<'a> {
    script: &'a Path,
    working_dir: &'a Path,
    lua_bin: &'a str,
    config: &'a Config,
    lua_path: &'a str,
    lua_cpath: &'a str,
    version_suffix: &'a str,
    gen_target: &'a str,
    gen_compat: &'a str,
}

impl Compiler<'_> {
    async fn compile(&self, source: &Path, target: &Path) -> Result<(), Error> {
        if let Some(parent) = target.parent() {
            fs::tokio::create_dir_all(parent).await?;
        }
        let args = vec![
            self.script.as_os_str().to_owned(),
            "-q".into(),
            "--gen-target".into(),
            self.gen_target.into(),
            "--gen-compat".into(),
            self.gen_compat.into(),
            "gen".into(),
            "--output".into(),
            target.as_os_str().to_owned(),
            source.as_os_str().to_owned(),
        ];
        let output = self
            .config
            .wrapped_command(self.lua_bin, args)
            .current_dir(self.working_dir)
            .env_remove("TL_PATH")
            .env("LUA_PATH", self.lua_path)
            .env(format!("LUA_PATH_{}", self.version_suffix), self.lua_path)
            .env("LUA_CPATH", self.lua_cpath)
            .env(format!("LUA_CPATH_{}", self.version_suffix), self.lua_cpath)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|source| Error::Run { source })?;
        if !output.status.success() {
            return Err(Error::Compile {
                file: source.to_path_buf(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        fs::make_writable(target).await?;
        Ok(())
    }
}

fn teal_script(build_tree: &tree::Tree) -> Option<PathBuf> {
    [
        build_tree.unwrapped_bin().join("tl"),
        build_tree.bin().join("tl"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn gen_options(target_version: &lua::Version) -> (&'static str, &'static str) {
    match target_version {
        lua::Version::Lua53 => ("5.3", "optional"),
        lua::Version::Lua54 => ("5.4", "off"),
        lua::Version::Lua55 => ("5.5", "off"),
        _ => ("5.1", "optional"),
    }
}

fn lua_path(build_dir: &Path, build_paths: &Paths, target_paths: &Paths) -> String {
    let mut entries = vec![
        build_paths.package_path().joined(),
        target_paths.package_path().joined(),
    ];
    for root in ["src", "lua", "lib"] {
        entries.push(
            build_dir
                .join(root)
                .join("?.lua")
                .to_slash_lossy()
                .into_owned(),
        );
        entries.push(
            build_dir
                .join(root)
                .join("?")
                .join("init.lua")
                .to_slash_lossy()
                .into_owned(),
        );
    }
    entries.retain(|entry| !entry.is_empty());
    entries.join(";")
}

fn is_declaration(file: &Path) -> bool {
    file.extension().is_some_and(|ext| ext == "tl")
        && file
            .file_stem()
            .is_some_and(|stem| Path::new(stem).extension().is_some_and(|ext| ext == "d"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_teal_declarations() {
        assert!(is_declaration(Path::new("src/foo.d.tl")));
        assert!(is_declaration(Path::new("src/foo/bar.d.tl")));
        assert!(!is_declaration(Path::new("src/foo.tl")));
        assert!(!is_declaration(Path::new("src/foo.d.lua")));
    }
}
