use crate::{
    build::utils::c_dylib_extension,
    config::Config,
    lua_version::LuaVersion,
    tree::{InstallTree, TreeError},
};
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathBufExt;
use serde::Serialize;
use std::{env, fmt::Display, path::PathBuf, str::FromStr};
use thiserror::Error;

const LUA_PATH_SEPARATOR: &str = ";";
const LUA_INIT: &str = "require('lux').loader()";

/// Whether Lua should only see the current tree's packages (pure mode)
/// or also user-installed packages and system paths (impure).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LuaEnvironment {
    #[default]
    Pure,
    Impure,
}

#[derive(PartialEq, Eq, Debug, Serialize)]
pub struct Paths {
    /// Paths for Lua libraries
    src: PackagePath,
    /// Paths for native Lua libraries
    lib: PackagePath,
    /// Paths for executables
    bin: BinPath,

    version: LuaVersion,
}

#[derive(Debug, Error, Diagnostic)]
pub enum PathsError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
}

impl Paths {
    fn default(tree: &impl InstallTree) -> Self {
        Self {
            src: <_>::default(),
            lib: <_>::default(),
            bin: <_>::default(),
            version: tree.version().clone(),
        }
    }

    pub fn new(tree: &impl InstallTree) -> Result<Self, PathsError> {
        let mut paths = Self::default(tree);
        for package in tree.list()?.values().flatten() {
            let layout = tree.layout_for(&package.spec);
            paths.src.0.push(layout.src.join("?.lua"));
            paths.src.0.push(layout.src.join("?").join("init.lua"));
            paths
                .lib
                .0
                .push(layout.lib.join(format!("?.{}", c_dylib_extension())));
            paths.bin.add_path(tree.bin());
        }

        if let Some(lib_path) = tree.version().lux_lib_dir() {
            paths.prepend(&Paths {
                version: tree.version().clone(),
                src: <_>::default(),
                bin: <_>::default(),
                lib: PackagePath(vec![lib_path.join(format!("?.{}", c_dylib_extension()))]),
            });
        }

        Ok(paths)
    }

    pub fn for_environment<T: InstallTree>(
        tree: &T,
        config: &Config,
        environment: LuaEnvironment,
    ) -> Result<Self, PathsError> {
        let tree_paths = Self::new(tree)?;
        match environment {
            LuaEnvironment::Pure => Ok(tree_paths),
            LuaEnvironment::Impure => {
                let user_tree = config
                    .user_tree(tree.version().clone())
                    .map_err(PathsError::from)?;
                let mut paths = Self::new(&user_tree)?;
                paths.prepend(&tree_paths);
                Ok(paths)
            }
        }
    }

    /// Get the `package.path`
    pub fn package_path(&self) -> &PackagePath {
        &self.src
    }

    /// Get the `package.cpath`
    pub fn package_cpath(&self) -> &PackagePath {
        &self.lib
    }

    /// Get the `package.path`, prepended to `LUA_PATH`
    pub fn package_path_prepended(&self) -> PackagePath {
        let mut lua_path = PackagePath::from_str(env::var("LUA_PATH").unwrap_or_default().as_str())
            .unwrap_or_default();
        lua_path.prepend(self.package_path());
        lua_path
    }

    /// Get the `package.cpath`, prepended to `LUA_CPATH`
    pub fn package_cpath_prepended(&self) -> PackagePath {
        let mut lua_cpath =
            PackagePath::from_str(env::var("LUA_CPATH").unwrap_or_default().as_str())
                .unwrap_or_default();
        lua_cpath.prepend(self.package_cpath());
        lua_cpath
    }

    pub fn lua_path(&self, environment: LuaEnvironment) -> String {
        self.lua_package_path(self.package_path(), environment, "LUA_PATH")
    }

    pub fn lua_cpath(&self, environment: LuaEnvironment) -> String {
        self.lua_package_path(self.package_cpath(), environment, "LUA_CPATH")
    }

    pub fn lua_env_vars(
        &self,
        environment: LuaEnvironment,
        lua_init: &str,
    ) -> Vec<(String, String)> {
        let lua_path = self.lua_path(environment);
        let lua_cpath = self.lua_cpath(environment);
        vec![
            ("LUA_PATH".into(), lua_path.clone()),
            (self.versioned_env_var("LUA_PATH"), lua_path),
            ("LUA_CPATH".into(), lua_cpath.clone()),
            (self.versioned_env_var("LUA_CPATH"), lua_cpath),
            ("LUA_INIT".into(), lua_init.to_string()),
            (self.versioned_env_var("LUA_INIT"), lua_init.to_string()),
        ]
    }

    /// The Lua version these paths belong to.
    pub fn version(&self) -> &LuaVersion {
        &self.version
    }

    /// Get the `$PATH`
    pub fn path(&self) -> &BinPath {
        &self.bin
    }

    /// Get `$LUA_INIT`
    pub fn init(&self) -> String {
        format!("if _VERSION:find('{}') then {LUA_INIT} end", self.version)
    }

    /// Get the `$PATH`, prepended to the existing `$PATH` environment.
    pub fn path_prepended(&self) -> BinPath {
        let mut path = BinPath::from_env();
        path.prepend(self.path());
        path
    }

    pub fn prepend(&mut self, other: &Self) {
        self.src.prepend(&other.src);
        self.lib.prepend(&other.lib);
        self.bin.prepend(&other.bin);
    }
}

impl Paths {
    fn lua_package_path(
        &self,
        tree: &PackagePath,
        environment: LuaEnvironment,
        var: &str,
    ) -> String {
        match environment {
            LuaEnvironment::Pure => tree.joined(),
            LuaEnvironment::Impure => {
                let mut paths = self.inherited_package_path(var);
                paths.prepend(tree);
                format!("{};;", paths.joined())
            }
        }
    }

    fn inherited_package_path(&self, var: &str) -> PackagePath {
        env::var(self.versioned_env_var(var))
            .or_else(|_| env::var(var))
            .ok()
            .and_then(|value| PackagePath::from_str(&value).ok())
            .unwrap_or_default()
    }

    fn versioned_env_var(&self, var: &str) -> String {
        format!(
            "{var}_{}",
            self.version.version_compatibility_str().replace('.', "_")
        )
    }
}

#[cfg(test)]
impl Paths {
    pub(crate) fn test(version: LuaVersion, src: PackagePath, lib: PackagePath) -> Self {
        Self {
            src,
            lib,
            bin: BinPath::default(),
            version,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Default, Serialize, Clone)]
pub struct PackagePath(Vec<PathBuf>);

impl PackagePath {
    fn prepend(&mut self, other: &Self) {
        let mut new_vec = other.0.to_owned();
        new_vec.append(&mut self.0);
        self.0 = new_vec;
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn joined(&self) -> String {
        self.0
            .iter()
            .unique()
            .map(|path| path.to_slash_lossy())
            .join(LUA_PATH_SEPARATOR)
    }
}

impl FromStr for PackagePath {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let paths = s
            .trim_start_matches(LUA_PATH_SEPARATOR)
            .trim_end_matches(LUA_PATH_SEPARATOR)
            .split(LUA_PATH_SEPARATOR)
            .map(PathBuf::from)
            .collect();
        Ok(PackagePath(paths))
    }
}

impl Display for PackagePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.joined().fmt(f)
    }
}

#[derive(PartialEq, Eq, Debug, Default, Serialize)]
pub struct BinPath(Vec<PathBuf>);

impl BinPath {
    pub fn from_env() -> Self {
        Self::from_str(env::var("PATH").unwrap_or_default().as_str()).unwrap_or_default()
    }
    pub fn prepend(&mut self, other: &Self) {
        let mut new_vec = other.0.to_owned();
        new_vec.append(&mut self.0);
        self.0 = new_vec;
    }
    /// Adds a `PathBuf` to the path.
    /// Drops it silently if the `PathBuf` is invalid and cannot be used to create a `PATH` variable
    /// (e.g. if it contains `PATH` separator characters).
    pub fn add_path(&mut self, path: PathBuf) {
        if env::join_paths(vec![&path]).is_ok() {
            let mut new_vec = Vec::new();
            new_vec.push(path);
            new_vec.append(&mut self.0);
            self.0 = new_vec;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Joins the path to create a PATH expression.
    /// If a path contains invalid characters (e.g. the PATH separator),
    /// this returns an empty string.
    pub fn joined(&self) -> String {
        env::join_paths(self.0.iter().unique())
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }
}

impl FromStr for BinPath {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let paths = env::split_paths(s).collect();
        Ok(BinPath(paths))
    }
}

impl Display for BinPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.joined().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::ffi::OsString;

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = env::var_os(key);
            env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    fn test_paths(version: LuaVersion, src: &[&str], lib: &[&str]) -> Paths {
        Paths {
            src: PackagePath(src.iter().map(|path| PathBuf::from(*path)).collect()),
            lib: PackagePath(lib.iter().map(|path| PathBuf::from(*path)).collect()),
            bin: BinPath::default(),
            version,
        }
    }

    #[test]
    fn package_path_leading_trailing_delimiters() {
        let path = PackagePath::from_str(
            ";;/path/to/some/lib/lua/5.1/?.so;/path/to/another/lib/lua/5.1/?.so;;;",
        )
        .unwrap();
        assert_eq!(
            path,
            PackagePath(vec![
                "/path/to/some/lib/lua/5.1/?.so".into(),
                "/path/to/another/lib/lua/5.1/?.so".into(),
            ])
        );
        assert_eq!(
            format!("{path}"),
            "/path/to/some/lib/lua/5.1/?.so;/path/to/another/lib/lua/5.1/?.so"
        );
    }

    #[test]
    #[serial]
    fn pure_lua_path_ignores_inherited_paths() {
        let _base = EnvGuard::set("LUA_PATH", "/system/?.lua");
        let _versioned = EnvGuard::set("LUA_PATH_5_4", "/system54/?.lua");
        let paths = test_paths(LuaVersion::Lua54, &["/project/?.lua"], &[]);

        assert_eq!(paths.lua_path(LuaEnvironment::Pure), "/project/?.lua");
    }

    #[test]
    #[serial]
    fn impure_lua_path_prefers_versioned_env_and_appends_defaults() {
        let _base = EnvGuard::set("LUA_PATH", "/system/?.lua");
        let _versioned = EnvGuard::set("LUA_PATH_5_4", "/system54/?.lua");
        let paths = test_paths(LuaVersion::Lua54, &["/project/?.lua"], &[]);

        assert_eq!(
            paths.lua_path(LuaEnvironment::Impure),
            "/project/?.lua;/system54/?.lua;;"
        );
    }

    #[test]
    #[serial]
    fn impure_lua_path_falls_back_to_unversioned_env() {
        let _base = EnvGuard::set("LUA_PATH", "/system/?.lua");
        let _versioned = EnvGuard::set("LUA_PATH_5_3", "/system53/?.lua");
        let paths = test_paths(LuaVersion::Lua54, &["/project/?.lua"], &[]);

        assert_eq!(
            paths.lua_path(LuaEnvironment::Impure),
            "/project/?.lua;/system/?.lua;;"
        );
    }

    #[test]
    #[serial]
    fn impure_lua_cpath_merges_inherited_paths() {
        let _versioned = EnvGuard::set("LUA_CPATH_5_4", "/system54/?.so");
        let paths = test_paths(LuaVersion::Lua54, &[], &["/project/?.so"]);

        assert_eq!(
            paths.lua_cpath(LuaEnvironment::Impure),
            "/project/?.so;/system54/?.so;;"
        );
    }
}
