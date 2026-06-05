use std::path::PathBuf;

use serde::{de, Deserialize};

/// The `lux.toml` file for a workspace.
/// Used to deserialize a workspace with multiple projects.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct WorkspaceToml {
    pub workspace: WorkspaceSpec,
}

impl WorkspaceToml {
    pub(super) fn new(toml_content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_content)
    }
}

/// The `lux.toml` file for a workspace.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct WorkspaceSpec {
    pub members: Vec<WorkspaceMemberSpec>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub(super) enum WorkspaceMemberSpec {
    /// Glob for a path (relative to the workspace root) of projects to include in the workspace.
    RelativeProjectGlob(String),
    /// Path (relative to the workspace root) of projects to include in the workspace.
    RelativeProjectPath(PathBuf),
}

impl<'de> Deserialize<'de> for WorkspaceMemberSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path_or_glob = String::deserialize(deserializer)?;
        Ok(match path_or_glob.strip_prefix("glob:") {
            Some(pattern) => {
                // Fail to deserialize if glob can't parse the pattern
                let _ = glob::Pattern::new(pattern).map_err(de::Error::custom)?;
                WorkspaceMemberSpec::RelativeProjectGlob(pattern.into())
            }
            None => WorkspaceMemberSpec::RelativeProjectPath(PathBuf::from(path_or_glob)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_workspace_toml() {
        let toml_content = r#"
            [workspace]
            members = [
                "foo",
                "projects/bar",
                "glob:projects/baz/*",
            ]
        "#;
        let workspace_toml = WorkspaceToml::new(toml_content).unwrap();
        assert_eq!(
            workspace_toml.workspace.members,
            vec![
                WorkspaceMemberSpec::RelativeProjectPath(PathBuf::from("foo")),
                WorkspaceMemberSpec::RelativeProjectPath(PathBuf::from("projects/bar")),
                WorkspaceMemberSpec::RelativeProjectGlob("projects/baz/*".into()),
            ]
        );
    }
}
