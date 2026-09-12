use crate::{
    build::utils::format_path,
    config::Config,
    fs,
    lockfile::{LocalPackage, LocalPackageId, Lockfile, LockfileError, ReadOnly},
    lua_version::LuaVersion,
    package::{PackageName, PackageReq},
    variables::{GetVariableError, HasVariables},
};
use std::{collections::HashMap, io, path::PathBuf, sync::Arc};

use itertools::Itertools;
use miette::Diagnostic;
use nonempty::NonEmpty;
use thiserror::Error;
mod dist;
mod layout;
mod list;

pub use dist::*;
pub use layout::*;

const LOCKFILE_NAME: &str = "lux.lock";

/// A tree is a collection of files where installed rocks are located.
///
/// `lux` diverges from the traditional hierarchy employed by luarocks.
/// Instead, we opt for a much simpler approach:
///
/// - /rocks/<lua-version> - contains rocks
/// - /rocks/<lua-version>/<rock>/etc - documentation and supplementary files for the rock
/// - /rocks/<lua-version>/<rock>/lib - shared libraries (.so files)
/// - /rocks/<lua-version>/<rock>/src - library code for the rock
/// - /bin - binary files produced by various rocks
pub trait InstallTree {
    /// The Lua version for which to install packages.
    fn version(&self) -> &LuaVersion;
    /// The root directory of the tree
    fn root(&self) -> PathBuf;
    /// Where wrapped package binaries are installed
    fn bin(&self) -> PathBuf;
    /// Where unwrapped package binaries are installed
    fn unwrapped_bin(&self) -> PathBuf;
    /// The standard install layout for a package.
    fn layout_for(&self, package: &LocalPackage) -> RockLayout;
    /// Create a [`Lockfile`] for this tree.
    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, TreeError>;
    /// Get this tree's lockfile path.
    fn lockfile_path(&self) -> PathBuf;
    /// The tree in which to install build dependencies.
    fn build_tree(&self, config: &Config) -> Result<Tree, TreeError>;
    /// The tree in which to install test dependencies.
    fn test_tree(&self, config: &Config) -> Result<Tree, TreeError>;
    /// List the packages that are installed in this tree.
    fn list(&self) -> Result<HashMap<PackageName, Vec<LocalPackage>>, TreeError>;
    /// Find installed rocks that match the given [`PackageReq`].
    fn match_rocks(&self, req: &PackageReq) -> Result<RockMatches, TreeError>;
    /// Create the standard directories (src, lib, etc.) for a package.
    ///
    /// For entrypoints, this also applies the custom layout, if one is configured.
    fn prepare(&self, package: &LocalPackage, entry_type: EntryType) -> Result<(), TreeError>;
    /// Remove the install layout directories.
    ///
    /// For entrypoints, this also applies the custom layout, if one is configured.
    fn cleanup(&self, package: &LocalPackage, entry_type: EntryType) -> Result<(), TreeError>;
}

/// The standard install layout for a rock.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RockLayout {
    /// The local installation directory.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(PREFIX)`.
    pub root: PathBuf,
    /// The `etc` directory, containing resources.
    pub etc: PathBuf,
    /// The `lib` directory, containing native libraries.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(LIBDIR)`.
    pub lib: PathBuf,
    /// The `src` directory, containing Lua sources.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(LUADIR)`.
    pub src: PathBuf,
    /// The `bin` directory, containing executables.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(BINDIR)`.
    /// This points to a global binary path at the root of the current tree by default.
    pub bin: PathBuf,
    /// The `etc/conf` directory, containing configuration files.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(CONFDIR)`.
    pub conf: PathBuf,
    /// The `etc/doc` directory, containing documentation files.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(DOCDIR)`.
    pub doc: PathBuf,
}

impl RockLayout {
    /// Create the standard install layout rooted at `root`, with binaries in `bin`.
    pub fn new(root: PathBuf, bin: PathBuf) -> Self {
        let etc = root.join("etc");
        let lib = root.join("lib");
        let src = root.join("src");
        let conf = etc.join("conf");
        let doc = etc.join("doc");
        Self {
            root,
            etc,
            lib,
            src,
            bin,
            conf,
            doc,
        }
    }

    /// The path to the package's stored rockspec.
    pub fn rockspec_path(&self) -> PathBuf {
        self.root.join("package.rockspec")
    }
}

/// A Lux install tree that supports multiple versions of the same dependency,
/// with packages addressed by their [`LocalPackageId`]
#[derive(Clone, Debug)]
pub struct Tree {
    /// The Lua version of the tree.
    version: LuaVersion,
    /// The parent of this tree's root directory.
    root_parent: PathBuf,
    /// The root of this tree's test dependency tree.
    test_tree_dir: PathBuf,
    /// The root of this tree's build dependency tree.
    build_tree_dir: PathBuf,
    /// The custom layout to apply to entrypoint packages, if any.
    entrypoint_layout: Option<Arc<dyn CustomRockLayout>>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum TreeError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] LockfileError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl HasVariables for RockLayout {
    fn get_variable(&self, var: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match var {
            "PREFIX" => Some(format_path(&self.root)),
            "LIBDIR" => Some(format_path(&self.lib)),
            "LUADIR" => Some(format_path(&self.src)),
            "BINDIR" => Some(format_path(&self.bin)),
            "CONFDIR" => Some(format_path(&self.conf)),
            "DOCDIR" => Some(format_path(&self.doc)),
            _ => None,
        })
    }
}

impl Tree {
    /// NOTE: This is exposed for use by the config module.
    /// Use `Config::tree()`
    pub(crate) fn new(
        root: PathBuf,
        version: LuaVersion,
        config: &Config,
    ) -> Result<Self, TreeError> {
        let version_dir = root.join(version.to_string());
        let test_tree_dir = version_dir.join("test_dependencies");
        let build_tree_dir = version_dir.join("build_dependencies");
        Self::new_with_paths(root, test_tree_dir, build_tree_dir, version, config)
    }

    fn new_with_paths(
        root: PathBuf,
        test_tree_dir: PathBuf,
        build_tree_dir: PathBuf,
        version: LuaVersion,
        config: &Config,
    ) -> Result<Self, TreeError> {
        let path_with_version = root.join(version.to_string());

        // Ensure that the root and the version directory exist.
        fs::sync::create_dir_all(&path_with_version)?;

        // In case the tree is in a git repository, we tell git to ignore it.
        let gitignore_file = root.join(".gitignore");
        fs::sync::write(&gitignore_file, "*")?;

        // Ensure that the bin directory exists.
        let bin_dir = path_with_version.join("bin");
        fs::sync::create_dir_all(&bin_dir)?;

        Ok(Self {
            root_parent: root,
            version,
            test_tree_dir,
            build_tree_dir,
            entrypoint_layout: config.entrypoint_layout().cloned(),
        })
    }

    pub fn match_rocks_and<F>(&self, req: &PackageReq, filter: F) -> Result<RockMatches, TreeError>
    where
        F: Fn(&LocalPackage) -> bool,
    {
        match self.list()?.get(req.name()) {
            Some(packages) => {
                let found_packages = packages
                    .iter()
                    .rev()
                    .filter(|package| {
                        req.version_req().matches(package.version()) && filter(package)
                    })
                    .map(|package| package.id())
                    .collect_vec();

                Ok(match NonEmpty::try_from(found_packages) {
                    Ok(found_packages) => {
                        if found_packages.len() == 1 {
                            RockMatches::Single(found_packages.last().clone())
                        } else {
                            RockMatches::Many(found_packages)
                        }
                    }
                    Err(_) => RockMatches::NotFound(req.clone()),
                })
            }
            None => Ok(RockMatches::NotFound(req.clone())),
        }
    }
}

impl InstallTree for Tree {
    fn version(&self) -> &LuaVersion {
        &self.version
    }

    fn root(&self) -> PathBuf {
        self.root_parent.join(self.version.to_string())
    }

    fn prepare(&self, package: &LocalPackage, entry_type: EntryType) -> Result<(), TreeError> {
        let layout = self.layout_for(package);
        fs::sync::create_dir_all(&layout.root)?;
        fs::sync::create_dir_all(&layout.lib)?;
        fs::sync::create_dir_all(&layout.src)?;
        fs::sync::create_dir_all(&layout.etc)?;

        if entry_type.is_entrypoint() {
            if let Some(custom_layout) = &self.entrypoint_layout {
                custom_layout.make_symlinks(self, package)?;
            }
        }

        Ok(())
    }

    fn cleanup(&self, package: &LocalPackage, entry_type: EntryType) -> Result<(), TreeError> {
        if entry_type.is_entrypoint() {
            if let Some(layout) = &self.entrypoint_layout {
                layout.remove_symlinks(self, package)?;
            }
        }

        let layout = self.layout_for(package);
        fs::sync::remove_dir_all(&layout.etc)?;
        fs::sync::remove_dir_all(&layout.root)?;

        for relative_binary_path in package.spec.binaries() {
            if let Some(binary_file_name) = relative_binary_path.file_name() {
                let binary_path = self.bin().join(binary_file_name);
                if binary_path.is_file() {
                    fs::sync::remove_file(binary_path)?;
                }

                let unwrapped_binary_path = self.unwrapped_bin().join(binary_file_name);
                if unwrapped_binary_path.is_file() {
                    fs::sync::remove_file(unwrapped_binary_path)?;
                }
            }
        }

        Ok(())
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, TreeError> {
        Ok(Lockfile::new(self.lockfile_path())?)
    }

    fn lockfile_path(&self) -> PathBuf {
        self.root().join(LOCKFILE_NAME)
    }

    fn layout_for(&self, package: &LocalPackage) -> RockLayout {
        RockLayout::new(
            self.root().join(format!(
                "{}-{}@{}",
                package.id(),
                package.name(),
                package.version()
            )),
            self.bin(),
        )
    }

    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.bin().join("unwrapped")
    }

    fn test_tree(&self, config: &Config) -> Result<Self, TreeError> {
        let test_tree_dir = self.test_tree_dir.clone();
        let build_tree_dir = self.build_tree_dir.clone();
        Self::new_with_paths(
            test_tree_dir.clone(),
            test_tree_dir,
            build_tree_dir,
            self.version.clone(),
            config,
        )
    }

    fn build_tree(&self, config: &Config) -> Result<Self, TreeError> {
        let test_tree_dir = self.test_tree_dir.clone();
        let build_tree_dir = self.build_tree_dir.clone();
        Self::new_with_paths(
            build_tree_dir.clone(),
            test_tree_dir,
            build_tree_dir,
            self.version.clone(),
            config,
        )
    }

    fn list(&self) -> Result<HashMap<PackageName, Vec<LocalPackage>>, TreeError> {
        Ok(self.lockfile()?.list())
    }

    fn match_rocks(&self, req: &PackageReq) -> Result<RockMatches, TreeError> {
        let found_packages = self.lockfile()?.find_rocks(req);
        Ok(match NonEmpty::try_from(found_packages) {
            Ok(found_packages) => {
                if found_packages.len() == 1 {
                    RockMatches::Single(found_packages.last().clone())
                } else {
                    RockMatches::Many(found_packages)
                }
            }
            Err(_) => RockMatches::NotFound(req.clone()),
        })
    }
}

#[derive(Copy, Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub enum EntryType {
    Entrypoint,
    DependencyOnly,
}

impl EntryType {
    pub fn is_entrypoint(&self) -> bool {
        matches!(self, Self::Entrypoint)
    }
}

#[derive(Clone, Debug)]
pub enum RockMatches {
    NotFound(PackageReq),
    Single(LocalPackageId),
    Many(NonEmpty<LocalPackageId>),
}

// Loosely mimic the Option<T> functions.
impl RockMatches {
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Single(_) | Self::Many(_))
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::PathCopy;
    use itertools::Itertools;
    use std::path::PathBuf;

    use insta::assert_yaml_snapshot;

    use crate::{
        config::ConfigBuilder,
        lockfile::{LocalPackage, LocalPackageHashes, LockConstraint},
        lua_version::LuaVersion,
        package::{PackageName, PackageSpec, PackageVersion},
        remote_package_source::RemotePackageSource,
        rockspec::RockBinaries,
        tree::{EntryType, InstallTree, RockLayout},
        variables,
    };

    #[test]
    fn rock_layout() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();

        let mock_hashes = LocalPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        };

        let package = LocalPackage::from(
            &PackageSpec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            LockConstraint::Unconstrained,
            RockBinaries::default(),
            RemotePackageSource::Test,
            None,
            mock_hashes.clone(),
        );

        let id = package.id();
        tree.prepare(&package, EntryType::Entrypoint).unwrap();

        assert_eq!(
            tree.layout_for(&package),
            RockLayout {
                bin: tree_path.join("5.1/bin"),
                root: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1")),
                etc: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc")),
                lib: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/lib")),
                src: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/src")),
                conf: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc/conf")),
                doc: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc/doc")),
            }
        );
    }

    #[test]
    fn tree_list() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();
        let result = tree.list().unwrap();
        // note: sorted_redaction doesn't work because we have a nested Vec
        let sorted_result: Vec<(PackageName, Vec<PackageVersion>)> = result
            .into_iter()
            .sorted()
            .map(|(name, package)| {
                (
                    name,
                    package
                        .into_iter()
                        .map(|package| package.spec.version)
                        .sorted()
                        .collect_vec(),
                )
            })
            .collect_vec();

        assert_yaml_snapshot!(sorted_result)
    }

    #[test]
    fn rock_layout_substitute() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();

        let mock_hashes = LocalPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        };

        let package = LocalPackage::from(
            &PackageSpec::parse("neorg".into(), "8.0.0-1-1".into()).unwrap(),
            LockConstraint::Unconstrained,
            RockBinaries::default(),
            RemotePackageSource::Test,
            None,
            mock_hashes.clone(),
        );
        let layout = tree.layout_for(&package);
        let build_variables = vec![
            "$(PREFIX)",
            "$(LIBDIR)",
            "$(LUADIR)",
            "$(BINDIR)",
            "$(CONFDIR)",
            "$(DOCDIR)",
        ];
        let result: Vec<String> = build_variables
            .into_iter()
            .map(|var| variables::substitute(&[&layout], var))
            .try_collect()
            .unwrap();
        assert_eq!(
            result,
            vec![
                layout.root.to_string_lossy().to_string(),
                layout.lib.to_string_lossy().to_string(),
                layout.src.to_string_lossy().to_string(),
                layout.bin.to_string_lossy().to_string(),
                layout.conf.to_string_lossy().to_string(),
                layout.doc.to_string_lossy().to_string(),
            ]
        );
    }
}
