use std::{collections::HashMap, path::PathBuf};

use path_slash::PathBufExt as _;

use crate::{
    config::Config,
    lockfile::locked_package,
    lua,
    tree::{self, InstallTree},
    variables::{GetVariableError, HasVariables},
};

#[derive(Debug, Default)]
pub(crate) struct Layouts {
    rockdirs: HashMap<String, PathBuf>,
    lua_version: Option<String>,
}

impl Layouts {
    pub(crate) fn new<T: InstallTree>(
        package: &locked_package::Spec,
        lua: &lua::Installation,
        tree: &T,
        config: &Config,
    ) -> Result<Self, tree::Error> {
        let build_tree = tree.build_tree(config)?;
        let lockfile = build_tree.lockfile()?;
        let rockdirs = package
            .build_dependencies()
            .into_iter()
            .filter_map(|id| lockfile.get(id))
            .map(|dependency| {
                let layout = build_tree.layout_for(dependency.spec());
                (
                    format!("{}_ROCKDIR", dependency.name().to_string().to_uppercase()),
                    layout.root,
                )
            })
            .collect();
        Ok(Self {
            rockdirs,
            lua_version: Some(lua.version.version_compatibility_str()),
        })
    }
}

impl HasVariables for Layouts {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "LUA_VERSION" => self.lua_version.clone(),
            _ => self
                .rockdirs
                .get(input)
                .map(|path| path.to_slash_lossy().into_owned()),
        })
    }
}
