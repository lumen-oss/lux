use std::{io, path::PathBuf};

use bon::Builder;
use itertools::Itertools;
use thiserror::Error;

use crate::{
    config::{Config, LuaVersion, LuaVersionUnset},
    lua_rockspec::LuaModule,
    package::PackageReq,
    tree::TreeError,
};

/// A rocks module finder.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Which<'a> {
    #[builder(start_fn)]
    module: LuaModule,
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(field)]
    packages: Vec<PackageReq>,
}

impl<State> WhichBuilder<'_, State>
where
    State: which_builder::State,
{
    pub fn package(mut self, package: PackageReq) -> Self {
        self.packages.push(package);
        self
    }

    pub fn packages(mut self, packages: impl IntoIterator<Item = PackageReq>) -> Self {
        self.packages.extend(packages);
        self
    }

    pub fn search(self) -> Result<PathBuf, WhichError>
    where
        State: which_builder::IsComplete,
    {
        do_search(self._build())
    }
}

#[derive(Error, Debug)]
pub enum WhichError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error("lua module {0} not found.")]
    ModuleNotFound(LuaModule),
}

fn do_search(which: Which<'_>) -> Result<PathBuf, WhichError> {
    let config = which.config;
    let lua_version = LuaVersion::from_config(config)?;
    let tree = config.user_tree(lua_version.clone())?;
    let lockfile = tree.lockfile()?;
    let local_packages = if which.packages.is_empty() {
        lockfile
            .list()
            .into_iter()
            .flat_map(|(_, pkgs)| pkgs)
            .collect_vec()
    } else {
        which
            .packages
            .iter()
            .flat_map(|req| {
                lockfile
                    .find_rocks(req)
                    .into_iter()
                    .filter_map(|id| lockfile.get(&id))
                    .cloned()
                    .collect_vec()
            })
            .collect_vec()
    };
    local_packages
        .into_iter()
        .filter_map(|pkg| {
            let rock_layout = tree.installed_rock_layout(&pkg).ok()?;
            let lib_path = rock_layout.lib.join(which.module.to_lib_path());
            if lib_path.is_file() {
                return Some(lib_path);
            }
            let lua_path = rock_layout.src.join(which.module.to_lua_path());
            if lua_path.is_file() {
                return Some(lua_path);
            }
            let lua_path = rock_layout.src.join(which.module.to_lua_init_path());
            if lua_path.is_file() {
                return Some(lua_path);
            }
            None
        })
        .next()
        .ok_or(WhichError::ModuleNotFound(which.module))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::{ConfigBuilder, LuaVersion};
    use assert_fs::prelude::PathCopy;
    use std::{path::PathBuf, str::FromStr};

    #[tokio::test]
    async fn test_which() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");
        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();

        let result = Which::new(LuaModule::from_str("foo.bar").unwrap(), &config)
            .search()
            .unwrap();
        assert_eq!(result.file_name().unwrap().to_string_lossy(), "bar.lua");
        assert_eq!(
            result
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "foo"
        );
        let result = Which::new(LuaModule::from_str("bat.baz").unwrap(), &config)
            .search()
            .unwrap();
        assert_eq!(result.file_name().unwrap().to_string_lossy(), "baz.so");
        assert_eq!(
            result
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "bat"
        );
        let result = Which::new(LuaModule::from_str("foo.bar").unwrap(), &config)
            .package("lua-cjson".parse().unwrap())
            .search();
        assert!(matches!(result, Err(WhichError::ModuleNotFound(_))));
        let result = Which::new(LuaModule::from_str("foo.bar").unwrap(), &config)
            .package("neorg@8.1.1-1".parse().unwrap())
            .search();
        assert!(matches!(result, Err(WhichError::ModuleNotFound(_))));
    }
}
