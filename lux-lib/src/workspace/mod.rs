use std::{
    io,
    ops::Deref,
    path::{Path, PathBuf},
};

use glob::glob;
use itertools::Itertools;
use lets_find_up::{find_up_with, FindUpKind, FindUpOptions};
use nonempty::NonEmpty;
use path_slash::PathBufExt;
use thiserror::Error;

use crate::{
    config::Config,
    lockfile::{LockfileError, ReadOnly, WorkspaceLockfile},
    lua_rockspec::LuaVersionError,
    lua_version::LuaVersion,
    package::PackageName,
    project::{Project, ProjectError, PROJECT_TOML},
    tree::{InstallTree, Tree, TreeError},
    workspace::workspace_toml::{WorkspaceMemberSpec, WorkspaceToml},
};

pub mod workspace_toml;

pub const WORKSPACE_TOML: &str = PROJECT_TOML;
pub(crate) const LUX_DIR_NAME: &str = ".lux";
const LUARC: &str = ".luarc.json";
const EMMYRC: &str = ".emmyrc.json";

/// A newtype for the workspace root directory.
/// This is used to ensure that the workspace root is a valid project directory.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct WorkspaceRoot(PathBuf);

impl AsRef<Path> for WorkspaceRoot {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl Deref for WorkspaceRoot {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("cannot get current directory: {0}")]
    GetCwd(io::Error),
    #[error("error reading lux.toml at {0}:\n{1}")]
    ReadLuxTOML(String, io::Error),
    #[error("error deserializing workspace TOML:\n{0}")]
    TOML(String),
    #[error("no project found at '{0}'")]
    ProjectNotFound(PathBuf),
    #[error("glob error: '{0}'")]
    Glob(String),
    #[error("error deserializing project TOML:\n{0}")]
    Project(#[from] ProjectError),
    #[error("no project or workspace found")]
    NoWorkspaceOrProject,
    #[error("empty workspace at '{0}'")]
    EmptyWorkspace(PathBuf),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
    #[error("not a lux project or workspace directory:\n'{0}'")]
    NotAWorkspaceDir(PathBuf),
    #[error("package must be specified in a multi-project workspace")]
    NoPackageSpecified,
    #[error("package '{0}' not found in workspace '{1}'")]
    PackageNotFound(PackageName, WorkspaceRoot),
}

#[derive(Error, Debug)]
pub enum WorkspaceTreeError {
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    LuaVersionError(#[from] LuaVersionError),
}

#[derive(Clone, Debug)]
pub struct Workspace {
    root: WorkspaceRoot,
    members: NonEmpty<Project>,
}

// TODO: move lockfile from project to workspace

impl Workspace {
    pub fn current() -> Result<Option<Self>, WorkspaceError> {
        let cwd = std::env::current_dir().map_err(WorkspaceError::GetCwd)?;
        Self::from(&cwd)
    }

    pub fn current_or_err() -> Result<Self, WorkspaceError> {
        let cwd = std::env::current_dir().map_err(WorkspaceError::GetCwd)?;
        Self::current()?.ok_or(WorkspaceError::NotAWorkspaceDir(cwd))
    }

    /// The path where the root `lux.toml` resides.
    pub fn root(&self) -> &WorkspaceRoot {
        &self.root
    }

    /// The members of this workspace.
    pub fn members(&self) -> &NonEmpty<Project> {
        &self.members
    }

    /// Mutable reference to the members of this workspace.
    pub fn members_mut(&mut self) -> &mut NonEmpty<Project> {
        &mut self.members
    }

    /// Get a workspace member, defaulting to the first one if none is specified.
    /// Fails if a package name is specified, but not found.
    pub fn single_member_or_select(
        &self,
        name: &Option<PackageName>,
    ) -> Result<&Project, WorkspaceError> {
        match name {
            Some(name) => self
                .members()
                .iter()
                .find(|project| &project.toml().package == name)
                .ok_or_else(|| WorkspaceError::PackageNotFound(name.clone(), self.root.clone())),
            None => Ok(self.members().first()),
        }
    }

    /// Get a mutable workspace member, defaulting to the first one if none is specified.
    /// Fails if a package name is specified, but not found.
    pub fn single_member_or_select_mut(
        &mut self,
        package: &Option<PackageName>,
    ) -> Result<&mut Project, WorkspaceError> {
        match package.as_ref() {
            Some(package) => self.select_member_mut(package),
            None => self.single_member_mut(),
        }
    }

    /// Get the single member of this workspace, failing if it has multiple members.
    pub fn single_member(&self) -> Result<&Project, WorkspaceError> {
        if self.members().len() == 1 {
            Ok(self.members().first())
        } else {
            Err(WorkspaceError::NoPackageSpecified)
        }
    }

    /// Get the single mutable member of this workspace, failing if it has multiple members.
    pub fn single_member_mut(&mut self) -> Result<&mut Project, WorkspaceError> {
        if self.members().len() == 1 {
            Ok(self.members_mut().first_mut())
        } else {
            Err(WorkspaceError::NoPackageSpecified)
        }
    }

    /// Select a member of this workspace, failing if it is not found.
    pub fn select_member(&self, package: &PackageName) -> Result<&Project, WorkspaceError> {
        let workspace_root = self.root.clone();
        self.members()
            .iter()
            .find(|project| &project.toml().package == package)
            .ok_or_else(|| WorkspaceError::PackageNotFound(package.clone(), workspace_root))
    }

    /// Select a mutable member of this workspace, failing if it is not found.
    pub fn select_member_mut(
        &mut self,
        package: &PackageName,
    ) -> Result<&mut Project, WorkspaceError> {
        let workspace_root = self.root.clone();
        self.members_mut()
            .iter_mut()
            .find(|project| &project.toml().package == package)
            .ok_or_else(|| WorkspaceError::PackageNotFound(package.clone(), workspace_root))
    }

    /// Get the `lux.lock` lockfile path.
    pub fn lockfile_path(&self) -> PathBuf {
        self.root.join("lux.lock")
    }

    /// Get the `lux.lock` lockfile in the project root.
    pub fn lockfile(&self) -> Result<WorkspaceLockfile<ReadOnly>, WorkspaceError> {
        Ok(WorkspaceLockfile::new(self.lockfile_path())?)
    }

    /// Get the `lux.lock` lockfile in the project root, if present.
    pub fn try_lockfile(&self) -> Result<Option<WorkspaceLockfile<ReadOnly>>, WorkspaceError> {
        let path = self.lockfile_path();
        if path.is_file() {
            Ok(Some(WorkspaceLockfile::load(path)?))
        } else {
            Ok(None)
        }
    }

    pub fn tree(&self, config: &Config) -> Result<Tree, WorkspaceTreeError> {
        self.lua_version_tree(self.lua_version(config)?, config)
    }

    pub fn lua_version(&self, config: &Config) -> Result<LuaVersion, LuaVersionError> {
        let mut lua_version = self.members().first().lua_version(config)?;
        // Ensure the lua version specified by the config matches all projects
        for project in self.members() {
            lua_version = project.lua_version(config)?;
        }
        Ok(lua_version)
    }

    pub(crate) fn lua_version_tree(
        &self,
        lua_version: LuaVersion,
        config: &Config,
    ) -> Result<Tree, WorkspaceTreeError> {
        Ok(Tree::new(
            self.default_tree_root_dir(),
            lua_version,
            config,
        )?)
    }

    pub(crate) fn default_tree_root_dir(&self) -> PathBuf {
        self.root.join(LUX_DIR_NAME)
    }

    pub fn test_tree(&self, config: &Config) -> Result<Tree, WorkspaceTreeError> {
        Ok(self.tree(config)?.test_tree(config)?)
    }

    pub fn build_tree(&self, config: &Config) -> Result<Tree, WorkspaceTreeError> {
        Ok(self.tree(config)?.build_tree(config)?)
    }

    /// Get the `.luarc.json` or `.emmyrc.json` path.
    pub fn luarc_path(&self) -> PathBuf {
        let luarc_path = self.root.join(LUARC);
        if luarc_path.is_file() {
            luarc_path
        } else {
            let emmy_path = self.root.join(EMMYRC);
            if emmy_path.is_file() {
                emmy_path
            } else {
                luarc_path
            }
        }
    }

    pub fn from_exact(start: impl AsRef<Path>) -> Result<Option<Self>, WorkspaceError> {
        if !start.as_ref().exists() {
            return Ok(None);
        }
        if start.as_ref().join(WORKSPACE_TOML).exists() {
            let toml_path = start.as_ref().join(WORKSPACE_TOML);
            let toml_content = std::fs::read_to_string(&toml_path).map_err(|err| {
                WorkspaceError::ReadLuxTOML(toml_path.to_string_lossy().to_string(), err)
            })?;
            let root = start.as_ref();
            let toml_obj: Option<toml::Table> = toml::from_str(&toml_content).ok();
            if toml_obj.is_some_and(|toml| toml.contains_key("workspace")) {
                Ok(Some(Self::from_toml(&toml_content, root)?))
            } else {
                let project =
                    Project::from_exact(root)?.ok_or(WorkspaceError::NoWorkspaceOrProject)?;
                Ok(Some(Workspace {
                    root: WorkspaceRoot(root.to_path_buf()),
                    members: NonEmpty::new(project),
                }))
            }
        } else {
            Ok(None)
        }
    }

    pub fn from(start: impl AsRef<Path>) -> Result<Option<Self>, WorkspaceError> {
        if !start.as_ref().exists() {
            return Ok(None);
        }
        match find_up_with(
            WORKSPACE_TOML,
            FindUpOptions {
                cwd: start.as_ref(),
                kind: FindUpKind::File,
            },
        ) {
            Ok(Some(path)) => {
                if let Some(root) = path.parent() {
                    let toml_content = std::fs::read_to_string(&path).map_err(|err| {
                        WorkspaceError::ReadLuxTOML(path.to_string_lossy().to_string(), err)
                    })?;
                    let toml_obj: Option<toml::Table> = toml::from_str(&toml_content).ok();
                    if toml_obj.is_some_and(|toml| toml.contains_key("workspace")) {
                        Ok(Some(Self::from_toml(&toml_content, root)?))
                    } else {
                        if let Some(parent) = root.parent() {
                            match Self::from(parent)? {
                                Some(workspace) => Ok(Some(workspace)),
                                None => {
                                    let project = Project::from_exact(root)?
                                        .ok_or(WorkspaceError::NoWorkspaceOrProject)?;
                                    Ok(Some(Workspace {
                                        root: WorkspaceRoot(root.to_path_buf()),
                                        members: NonEmpty::new(project),
                                    }))
                                }
                            }
                        } else {
                            Ok(None)
                        }
                    }
                } else {
                    Ok(None)
                }
            }
            // NOTE: If we hit a read error, it could be because we haven't found a PROJECT_TOML
            // or WORKSPACE_TOML and have started searching too far upwards.
            // See for example https://github.com/lumen-oss/lux/issues/532
            _ => Ok(None),
        }
    }

    fn from_toml(toml_content: &str, root: &Path) -> Result<Self, WorkspaceError> {
        let toml = WorkspaceToml::new(toml_content)
            .map_err(|err| WorkspaceError::TOML(err.to_string()))?;
        let mut members = Vec::new();
        for member in toml.workspace.members {
            match member {
                WorkspaceMemberSpec::RelativeProjectGlob(pattern) => {
                    let potential_paths = glob(root.join(pattern).to_slash_lossy().deref())
                        .ok() // This is fine because we fail to deserialize invalid globs
                        .into_iter()
                        .flat_map(|paths| {
                            paths.map(|path| {
                                path.map_err(|err| WorkspaceError::Glob(err.to_string()))
                            })
                        })
                        .try_collect::<_, Vec<_>, _>()?;
                    for project_path in potential_paths {
                        if let Some(project) = Project::from_exact(&project_path)? {
                            members.push(project)
                        }
                    }
                }
                WorkspaceMemberSpec::RelativeProjectPath(relative_project_path) => {
                    let project_path = root.join(relative_project_path);
                    match Project::from_exact(&project_path)? {
                        Some(project) => members.push(project),
                        None => return Err(WorkspaceError::ProjectNotFound(project_path)),
                    }
                }
            }
        }
        match NonEmpty::from_vec(members) {
            Some(members) => Ok(Workspace {
                root: WorkspaceRoot(root.to_path_buf()),
                members,
            }),
            None => Err(WorkspaceError::EmptyWorkspace(root.to_path_buf())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use assert_fs::prelude::PathCopy;

    #[tokio::test]
    async fn find_single_project_workspace() {
        let sample_project: PathBuf = "resources/test/sample-projects/init/".into();
        let project_root = assert_fs::TempDir::new().unwrap();
        project_root.copy_from(&sample_project, &["**"]).unwrap();
        let work_dir: PathBuf = project_root.join("src");
        let workspace = Workspace::from(&work_dir).unwrap().unwrap();
        assert_eq!(workspace.members.len(), 1);
        let project = workspace.members.first();
        assert_eq!(project.root().to_path_buf(), project_root.to_path_buf());
    }

    #[tokio::test]
    async fn find_multi_project_workspace() {
        let sample_workspace: PathBuf = "resources/test/sample-projects/multi-project/".into();
        let workspace_root = assert_fs::TempDir::new().unwrap();
        workspace_root
            .copy_from(&sample_workspace, &["**"])
            .unwrap();
        let work_dir: PathBuf = workspace_root.join("projects");
        let workspace = Workspace::from(&work_dir).unwrap().unwrap();
        assert_eq!(workspace.members.len(), 2);
        let foo = workspace.select_member(&"foo".into()).unwrap();
        assert_eq!(
            foo.root().to_path_buf(),
            workspace_root.join("projects/foo").to_path_buf()
        );
        let bar = workspace.select_member(&"bar".into()).unwrap();
        assert_eq!(
            bar.root().to_path_buf(),
            workspace_root.join("projects/bar").to_path_buf()
        );
    }

    #[tokio::test]
    async fn find_multi_project_workspace_members_glob() {
        let sample_workspace: PathBuf = "resources/test/sample-projects/multi-project/".into();
        let workspace_root = assert_fs::TempDir::new().unwrap();
        workspace_root
            .copy_from(&sample_workspace, &["**"])
            .unwrap();
        let work_dir: PathBuf = workspace_root.join("projects");
        let workspace_toml_file = workspace_root.join(WORKSPACE_TOML);
        let workspace_toml_content = r#"
[workspace]
members = [ "glob:projects/*" ]
"#;
        tokio::fs::write(&workspace_toml_file, workspace_toml_content)
            .await
            .unwrap();

        let workspace = Workspace::from(&work_dir).unwrap().unwrap();
        assert_eq!(workspace.members.len(), 2);
        let foo = workspace.select_member(&"foo".into()).unwrap();
        assert_eq!(
            foo.root().to_path_buf(),
            workspace_root.join("projects/foo").to_path_buf()
        );
        let bar = workspace.select_member(&"bar".into()).unwrap();
        assert_eq!(
            bar.root().to_path_buf(),
            workspace_root.join("projects/bar").to_path_buf()
        );
    }

    #[tokio::test]
    async fn test_no_find_workspace_upwards() {
        let work_dir = assert_fs::TempDir::new().unwrap();
        assert!(Workspace::from(&work_dir).unwrap().is_none())
    }
}
