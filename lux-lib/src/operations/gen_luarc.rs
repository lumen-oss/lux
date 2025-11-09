use crate::config::Config;
use crate::lockfile::LocalPackageLockType;
use crate::project::Project;
use crate::project::ProjectError;
use crate::project::ProjectTreeError;
use crate::project::LUX_DIR_NAME;
use bon::Builder;
use itertools::Itertools;
use path_slash::PathBufExt;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;

#[derive(Error, Debug)]
pub enum GenLuaRcError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    ProjectTree(#[from] ProjectTreeError),
    #[error("failed to serialize luarc content:\n{0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to write {0}:\n{1}")]
    Write(PathBuf, io::Error),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct GenLuaRc<'a> {
    config: &'a Config,
    project: &'a Project,
}

impl<State> GenLuaRcBuilder<'_, State>
where
    State: gen_lua_rc_builder::State + gen_lua_rc_builder::IsComplete,
{
    pub async fn generate_luarc(self) -> Result<(), GenLuaRcError> {
        do_generate_luarc(self._build()).await
    }
}

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
#[serde(default)]
struct LuaRC {
    #[serde(flatten)] // <-- capture any unknown keys here
    other: BTreeMap<String, serde_json::Value>,

    #[serde(default)]
    workspace: Workspace,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
struct Workspace {
    #[serde(flatten)] // <-- capture any unknown keys here
    other: BTreeMap<String, serde_json::Value>,

    #[serde(default)]
    library: Vec<String>,
}

async fn do_generate_luarc(args: GenLuaRc<'_>) -> Result<(), GenLuaRcError> {
    let config = args.config;
    if !config.generate_luarc() {
        return Ok(());
    }
    let project = args.project;
    let lockfile = project.lockfile()?;
    let luarc_path = project.luarc_path();

    // read the existing .luarc file or initialise a new one if it doesn't exist
    let luarc_content = fs::read_to_string(&luarc_path)
        .await
        .unwrap_or_else(|_| "{}".into());

    let dependency_tree = project.tree(config)?;
    let dependency_dirs = lockfile
        .local_pkg_lock(&LocalPackageLockType::Regular)
        .rocks()
        .values()
        .map(|dependency| dependency_tree.installed_rock_layout(dependency))
        .filter_map(Result::ok)
        .map(|rock_layout| rock_layout.src)
        .filter(|dir| dir.is_dir())
        .map(|dependency_dir| {
            diff_paths(dependency_dir, project.root())
                .expect("tree root should be a subpath of the project root")
        });

    let test_dependency_tree = project.test_tree(config)?;
    let test_dependency_dirs = lockfile
        .local_pkg_lock(&LocalPackageLockType::Test)
        .rocks()
        .values()
        .map(|dependency| test_dependency_tree.installed_rock_layout(dependency))
        .filter_map(Result::ok)
        .map(|rock_layout| rock_layout.src)
        .filter(|dir| dir.is_dir())
        .map(|test_dependency_dir| {
            diff_paths(test_dependency_dir, project.root())
                .expect("test tree root should be a subpath of the project root")
        });

    let addon_dirs = lockfile
        .addons()
        .iter()
        .flat_map(|addon| addon.library_paths.iter())
        .filter_map(|library_path| {
            let path = PathBuf::from(library_path);
            diff_paths(path, project.root())
        });

    let library_dirs = dependency_dirs
        .chain(test_dependency_dirs)
        .chain(addon_dirs)
        .sorted()
        .collect_vec();

    let luarc_content = update_luarc_content(&luarc_content, library_dirs)?;

    fs::write(&luarc_path, luarc_content)
        .await
        .map_err(|err| GenLuaRcError::Write(luarc_path, err))?;

    Ok(())
}

fn update_luarc_content(
    prev_contents: &str,
    extra_paths: Vec<PathBuf>,
) -> Result<String, GenLuaRcError> {
    let mut luarc: LuaRC = serde_json::from_str(prev_contents).unwrap();

    // remove any preexisting lux library paths
    luarc
        .workspace
        .library
        .retain(|path| !path.starts_with(&format!("{LUX_DIR_NAME}/")));

    extra_paths
        .iter()
        .map(|path| path.to_slash_lossy().to_string())
        .for_each(|path_str| luarc.workspace.library.push(path_str));

    Ok(serde_json::to_string_pretty(&luarc)?)
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_generate_luarc_with_previous_libraries_parametrized() {
        let cases = vec![
            (
                "Empty existing libraries, adding single lib", // 📝 Description
                r#"{
                    "workspace": {
                        "library": []
                    }
                }"#,
                vec![".lux/5.1/my-lib".into()],
                r#"{
                    "workspace": {
                        "library": [".lux/5.1/my-lib"]
                    }
                }"#,
            ),
            (
                "Other fields present, adding libs", // 📝 Description
                r#"{
                    "any-other-field": true,
                    "workspace": {
                        "library": []
                    }
                }"#,
                vec![".lux/5.1/lib-A".into(), ".lux/5.1/lib-B".into()],
                r#"{
                    "any-other-field": true,
                    "workspace": {
                        "library": [".lux/5.1/lib-A", ".lux/5.1/lib-B"]
                    }
                }"#,
            ),
            (
                "Removes not present libs, without removing others", // 📝 Description
                r#"{
                    "workspace": {
                        "library": [".lux/5.1/lib-A", ".lux/5.4/lib-B"]
                    }
                }"#,
                vec![".lux/5.1/lib-C".into()],
                r#"{
                    "workspace": {
                        "library": [".lux/5.1/lib-C"]
                    }
                }"#,
            ),
        ];

        for (description, initial, new_libs, expected) in cases {
            let content = super::update_luarc_content(initial, new_libs.clone()).unwrap();

            assert_eq!(
                serde_json::from_str::<LuaRC>(&content).unwrap(),
                serde_json::from_str::<LuaRC>(expected).unwrap(),
                "Case failed: {}\nInitial input:\n{}\nNew libs: {:?}",
                description,
                initial,
                &new_libs
            );
        }
    }
}
