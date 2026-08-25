use crate::{
    config::Config,
    fs,
    lockfile::LocalPackageLockType,
    lua_version::LuaVersion,
    tree::InstallTree,
    workspace::{Workspace, WorkspaceError, WorkspaceTreeError, LUX_DIR_NAME},
};
use bon::Builder;
use miette::Diagnostic;
use path_slash::PathBufExt;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

const LUAURC: &str = ".luaurc";

#[derive(Error, Debug, Diagnostic)]
pub enum GenLuauRcError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error("failed to serialize .luaurc content:\n{0}")]
    Serialize(String),
    #[error("failed to deserialize .luaurc content:\n{0}")]
    Deserialize(String),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct GenLuauRc<'a> {
    config: &'a Config,
    workspace: &'a Workspace,
}

impl<State> GenLuauRcBuilder<'_, State>
where
    State: gen_luau_rc_builder::State + gen_luau_rc_builder::IsComplete,
{
    /// Generates a .luaurc in the workspace root.
    /// No-op if the specified workspace tree is not a Luau tree.
    pub async fn generate_luau_rc(self) -> Result<(), GenLuauRcError> {
        do_generate_luau_rc(self._build()).await
    }
}

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
#[serde(default)]
struct LuauRc {
    #[serde(flatten)] // <-- capture any unknown keys here
    other: BTreeMap<String, serde_json::Value>,

    #[serde(default)]
    aliases: BTreeMap<String, String>,
}

async fn do_generate_luau_rc(args: GenLuauRc<'_>) -> Result<(), GenLuauRcError> {
    let workspace = args.workspace;
    let tree = workspace.tree(args.config)?;
    if tree.version() != &LuaVersion::Luau {
        return Ok(());
    }
    let lockfile = workspace.lockfile()?;
    let luaurc_path = workspace.root().join(LUAURC);

    let prev_content = fs::tokio::read_to_string(&luaurc_path)
        .await
        .unwrap_or_else(|_| "{}".into());

    let test_tree = workspace.test_tree(args.config)?;
    let mut aliases = BTreeMap::new();
    for (lock_type, tree) in [
        (LocalPackageLockType::Regular, Some(&tree)),
        (LocalPackageLockType::Test, Some(&test_tree)),
    ] {
        let Some(tree) = tree else {
            continue;
        };
        for package in lockfile.local_pkg_lock(&lock_type).rocks().values() {
            let layout = tree.layout_for(package);
            if layout.src.is_dir() {
                if let Some(rel_path) = diff_paths(&layout.src, workspace.root()) {
                    // NOTE: luau only resolves alias targets that are explicitly
                    // relative (`./`/`../`) or absolute
                    aliases.insert(
                        package.name().to_string(),
                        format!("./{}", rel_path.to_slash_lossy()),
                    );
                };
            }
        }
    }

    let luaurc_content = update_luau_rc_content(&prev_content, aliases)?;

    fs::tokio::write(&luaurc_path, luaurc_content).await?;

    Ok(())
}

fn update_luau_rc_content(
    prev_contents: &str,
    aliases: BTreeMap<String, String>,
) -> Result<String, GenLuauRcError> {
    let mut luaurc: LuauRc = serde_json::from_str(prev_contents)
        .map_err(|err| GenLuauRcError::Deserialize(err.to_string()))?;

    // remove any preexisting lux-managed aliases
    luaurc.aliases.retain(|_, path| {
        !path
            .strip_prefix("./")
            .unwrap_or(path)
            .starts_with(&format!("{LUX_DIR_NAME}/"))
    });

    luaurc.aliases.extend(aliases);

    serde_json::to_string_pretty(&luaurc).map_err(|err| GenLuauRcError::Serialize(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_luau_rc_content() {
        let cases = vec![
            (
                "empty previous content, adding single alias",
                "{}",
                BTreeMap::from([("my-lib".into(), "./.lux/luau/lib-1.0.0/my-lib/src".into())]),
                r#"{
                    "aliases": {
                        "my-lib": "./.lux/luau/lib-1.0.0/my-lib/src"
                    }
                }"#,
            ),
            (
                "other fields preserved, lux-managed aliases refreshed",
                r#"{
                    "language": { "type": "luau" },
                    "aliases": {
                        "old": ".lux/luau/old/src",
                        "user-alias": "../shared"
                    }
                }"#,
                BTreeMap::from([("new-lib".into(), "./.lux/luau/new/src".into())]),
                r#"{
                    "language": { "type": "luau" },
                    "aliases": {
                        "new-lib": "./.lux/luau/new/src",
                        "user-alias": "../shared"
                    }
                }"#,
            ),
        ];

        for (description, initial, new_aliases, expected) in cases {
            let content = super::update_luau_rc_content(initial, new_aliases.clone()).unwrap();
            assert_eq!(
                serde_json::from_str::<LuauRc>(&content).unwrap(),
                serde_json::from_str::<LuauRc>(expected).unwrap(),
                "Case failed: {}\nInitial input:\n{}\nNew aliases: {:?}",
                description,
                initial,
                new_aliases
            );
        }
    }
}
