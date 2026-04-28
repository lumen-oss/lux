use std::path::PathBuf;

use serde::Deserialize;

/// The `lux.toml` file for a workspace.
/// Used to deserialize a workspace with multiple projects.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct WorkspaceToml {
    pub workspace: WorkspaceSpec,
}

/// The `lux.toml` file for a workspace.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct WorkspaceSpec {
    /// Paths (relative to the workspace root) of projects to include in the workspace.
    pub members: Vec<PathBuf>,
}

impl WorkspaceToml {
    pub(super) fn new(toml_content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_content)
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
            ]
        "#;
        let workspace_toml = WorkspaceToml::new(toml_content).unwrap();
        assert_eq!(
            workspace_toml.workspace.members,
            vec![PathBuf::from("foo"), PathBuf::from("projects/bar")]
        );
    }
}
