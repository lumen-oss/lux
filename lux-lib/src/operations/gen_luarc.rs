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

    // Read any optional overrides from lux.toml to allow non-src library dirs for LuaLS
    let luarc_overrides = read_luarc_dependency_overrides(project);

    let dependency_tree = project.tree(config)?;
    let dependency_dirs = lockfile
        .local_pkg_lock(&LocalPackageLockType::Regular)
        .rocks()
        .values()
        .flat_map(|dependency| {
            let name = dependency.name().to_string();
            let override_dirs = luarc_overrides.get(&name).cloned();
            dependency_tree
                .installed_rock_layout(dependency)
                .ok()
                .into_iter()
                .flat_map(move |rock_layout| library_dirs_for(&rock_layout, override_dirs.as_ref()))
        })
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
        .flat_map(|dependency| {
            let name = dependency.name().to_string();
            let override_dirs = luarc_overrides.get(&name).cloned();
            test_dependency_tree
                .installed_rock_layout(dependency)
                .ok()
                .into_iter()
                .flat_map(move |rock_layout| library_dirs_for(&rock_layout, override_dirs.as_ref()))
        })
        .filter(|dir| dir.is_dir())
        .map(|test_dependency_dir| {
            diff_paths(test_dependency_dir, project.root())
                .expect("test tree root should be a subpath of the project root")
        });

    let library_dirs = dependency_dirs
        .chain(test_dependency_dirs)
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

/// Read optional per-dependency overrides from `lux.toml` that instruct `.luarc.json`
/// generation to include non-default library directories (e.g. `etc`).
fn read_luarc_dependency_overrides(
    project: &Project,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut map = std::collections::HashMap::<String, Vec<String>>::new();
    let toml = project.toml();
    let mut collect = |deps: &Option<Vec<crate::rockspec::lua_dependency::LuaDependencySpec>>| {
        if let Some(deps) = deps {
            for dep in deps {
                if let Some(luarc) = dep.luarc() {
                    if !luarc.is_empty() {
                        map.insert(dep.name().to_string(), luarc.clone());
                    }
                }
            }
        }
    };
    collect(&toml.dependencies);
    collect(&toml.test_dependencies);
    collect(&toml.build_dependencies);
    map
}

/// Given a rock layout and optional override directory keys, return library directories
/// to add to `.luarc.json`. Defaults to `src` when no override is present.
fn library_dirs_for(
    rock_layout: &crate::tree::RockLayout,
    override_dirs: Option<&Vec<String>>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match override_dirs {
        Some(keys) if !keys.is_empty() => {
            for key in keys {
                if key == "src" {
                    dirs.push(rock_layout.src.clone());
                } else if let Some(rest) = key.strip_prefix("src/") {
                    dirs.push(rock_layout.src.join(rest));
                } else if key == "etc" {
                    dirs.push(rock_layout.etc.clone());
                } else if let Some(rest) = key.strip_prefix("etc/") {
                    dirs.push(rock_layout.etc.join(rest));
                } else if key == "lib" {
                    dirs.push(rock_layout.lib.clone());
                } else if let Some(rest) = key.strip_prefix("lib/") {
                    dirs.push(rock_layout.lib.join(rest));
                } else if key == "doc" {
                    dirs.push(rock_layout.doc.clone());
                } else if let Some(rest) = key.strip_prefix("doc/") {
                    dirs.push(rock_layout.doc.join(rest));
                } else if key == "conf" {
                    dirs.push(rock_layout.conf.clone());
                } else if let Some(rest) = key.strip_prefix("conf/") {
                    dirs.push(rock_layout.conf.join(rest));
                } else {
                    // Default to a path under src
                    dirs.push(rock_layout.src.join(key));
                }
            }
        }
        _ => dirs.push(rock_layout.src.clone()),
    }
    dirs
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
