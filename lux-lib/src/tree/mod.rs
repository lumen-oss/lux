use crate::{
    build::utils::format_path,
    config::{tree::RockLayoutConfig, Config, LuaVersion},
    lockfile::{LocalPackage, LocalPackageId, Lockfile, LockfileError, OptState, ReadOnly},
    package::PackageReq,
    variables::{GetVariableError, HasVariables},
};
use std::{io, path::PathBuf};

use itertools::Itertools;
use mlua::{ExternalResult, IntoLua};
use nonempty::NonEmpty;
use thiserror::Error;

mod list;

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

#[derive(Clone, Debug)]
pub struct Tree {
    /// The Lua version of the tree.
    version: LuaVersion,
    /// The parent of this tree's root directory.
    root_parent: PathBuf,
    /// The rock layout config for this tree
    entrypoint_layout: RockLayoutConfig,
    /// The root of this tree's test dependency tree.
    test_tree_dir: PathBuf,
    /// The root of this tree's build dependency tree.
    build_tree_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum TreeError {
    #[error("unable to create directory {0}:\n{1}")]
    CreateDir(String, io::Error),
    #[error("unable to write to {0}:\n{1}")]
    WriteFile(String, io::Error),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
}

/// Change-agnostic way of referencing various paths for a rock.
#[derive(Debug, PartialEq)]
pub struct RockLayout {
    /// The local installation directory.
    /// Can be substituted in a rockspec's `build.build_variables` and `build.install_variables`
    /// using `$(PREFIX)`.
    pub rock_path: PathBuf,
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
    pub fn rockspec_path(&self) -> PathBuf {
        self.rock_path.join("package.rockspec")
    }
}

impl HasVariables for RockLayout {
    fn get_variable(&self, var: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match var {
            "PREFIX" => Some(format_path(&self.rock_path)),
            "LIBDIR" => Some(format_path(&self.lib)),
            "LUADIR" => Some(format_path(&self.src)),
            "BINDIR" => Some(format_path(&self.bin)),
            "CONFDIR" => Some(format_path(&self.conf)),
            "DOCDIR" => Some(format_path(&self.doc)),
            _ => None,
        })
    }
}

impl mlua::UserData for RockLayout {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("rock_path", |_, this| Ok(this.rock_path.clone()));
        fields.add_field_method_get("etc", |_, this| Ok(this.etc.clone()));
        fields.add_field_method_get("lib", |_, this| Ok(this.lib.clone()));
        fields.add_field_method_get("src", |_, this| Ok(this.src.clone()));
        fields.add_field_method_get("bin", |_, this| Ok(this.bin.clone()));
        fields.add_field_method_get("conf", |_, this| Ok(this.conf.clone()));
        fields.add_field_method_get("doc", |_, this| Ok(this.doc.clone()));
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
        std::fs::create_dir_all(&path_with_version).map_err(|err| {
            TreeError::CreateDir(path_with_version.to_string_lossy().to_string(), err)
        })?;

        // In case the tree is in a git repository, we tell git to ignore it.
        let gitignore_file = root.join(".gitignore");
        std::fs::write(&gitignore_file, "*").map_err(|err| {
            TreeError::WriteFile(gitignore_file.to_string_lossy().to_string(), err)
        })?;

        // Ensure that the bin directory exists.
        let bin_dir = path_with_version.join("bin");
        std::fs::create_dir_all(&bin_dir)
            .map_err(|err| TreeError::CreateDir(bin_dir.to_string_lossy().to_string(), err))?;

        let lockfile_path = root.join(LOCKFILE_NAME);
        let rock_layout_config = if lockfile_path.is_file() {
            let lockfile = Lockfile::load(lockfile_path, None)?;
            lockfile.entrypoint_layout
        } else {
            config.entrypoint_layout().clone()
        };
        Ok(Self {
            root_parent: root,
            version,
            entrypoint_layout: rock_layout_config,
            test_tree_dir,
            build_tree_dir,
        })
    }

    /// The root of the tree
    pub fn root(&self) -> PathBuf {
        self.root_parent.join(self.version.to_string())
    }

    pub fn version(&self) -> &LuaVersion {
        &self.version
    }

    pub fn root_for(&self, package: &LocalPackage) -> PathBuf {
        self.root().join(format!(
            "{}-{}@{}",
            package.id(),
            package.name(),
            package.version()
        ))
    }

    pub fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    /// Directory containing unwrapped Lua scripts
    /// The wrapped scripts are in `Self::bin()`
    pub(crate) fn unwrapped_bin(&self) -> PathBuf {
        self.bin().join("unwrapped")
    }

    pub fn match_rocks(&self, req: &PackageReq) -> Result<RockMatches, TreeError> {
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

    /// Get the `RockLayout` for an installed package.
    pub fn installed_rock_layout(&self, package: &LocalPackage) -> Result<RockLayout, TreeError> {
        let lockfile = self.lockfile()?;
        if lockfile.is_entrypoint(&package.id()) {
            Ok(self.entrypoint_layout(package))
        } else {
            Ok(self.dependency_layout(package))
        }
    }

    /// Create a `RockLayout` for an entrypoint
    pub fn entrypoint_layout(&self, package: &LocalPackage) -> RockLayout {
        self.mk_rock_layout(package, &self.entrypoint_layout)
    }

    /// Create a `RockLayout` for a dependency
    pub fn dependency_layout(&self, package: &LocalPackage) -> RockLayout {
        self.mk_rock_layout(package, &RockLayoutConfig::default())
    }

    /// Create a `RockLayout` for a package.
    fn mk_rock_layout(
        &self,
        package: &LocalPackage,
        layout_config: &RockLayoutConfig,
    ) -> RockLayout {
        let rock_path = self.root_for(package);
        let bin = self.bin();
        let etc_root = match layout_config.etc_root {
            Some(ref etc_root) => self.root().join(etc_root),
            None => rock_path.clone(),
        };
        let mut etc = match package.spec.opt {
            OptState::Required => etc_root.join(&layout_config.etc),
            OptState::Optional => etc_root.join(&layout_config.opt_etc),
        };
        if layout_config.etc_root.is_some() {
            etc = etc.join(format!("{}", package.name()));
        }
        let lib = rock_path.join("lib");
        let src = rock_path.join("src");
        let conf = etc.join(&layout_config.conf);
        let doc = etc.join(&layout_config.doc);

        RockLayout {
            rock_path,
            etc,
            lib,
            src,
            bin,
            conf,
            doc,
        }
    }

    /// Create a `RockLayout` for an entrypoint package, creating the `lib` and `src` directories.
    pub fn entrypoint(&self, package: &LocalPackage) -> io::Result<RockLayout> {
        let rock_layout = self.entrypoint_layout(package);
        std::fs::create_dir_all(&rock_layout.lib)?;
        std::fs::create_dir_all(&rock_layout.src)?;
        Ok(rock_layout)
    }

    /// Create a `RockLayout` for a dependency package, creating the `lib` and `src` directories.
    pub fn dependency(&self, package: &LocalPackage) -> io::Result<RockLayout> {
        let rock_layout = self.dependency_layout(package);
        std::fs::create_dir_all(&rock_layout.lib)?;
        std::fs::create_dir_all(&rock_layout.src)?;
        Ok(rock_layout)
    }

    pub fn lockfile(&self) -> Result<Lockfile<ReadOnly>, TreeError> {
        Ok(Lockfile::new(
            self.lockfile_path(),
            self.entrypoint_layout.clone(),
        )?)
    }

    /// Get this tree's lockfile path.
    pub fn lockfile_path(&self) -> PathBuf {
        self.root().join(LOCKFILE_NAME)
    }

    /// The tree in which to install test dependencies
    pub fn test_tree(&self, config: &Config) -> Result<Self, TreeError> {
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

    /// The tree in which to install build dependencies
    pub fn build_tree(&self, config: &Config) -> Result<Self, TreeError> {
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
}

impl mlua::UserData for Tree {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("root", |_, this, ()| Ok(this.root()));
        methods.add_method("root_for", |_, this, package: LocalPackage| {
            Ok(this.root_for(&package))
        });
        methods.add_method("bin", |_, this, ()| Ok(this.bin()));
        methods.add_method("match_rocks", |_, this, req: PackageReq| {
            this.match_rocks(&req)
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
        methods.add_method(
            "match_rock_and",
            |_, this, (req, callback): (PackageReq, mlua::Function)| {
                this.match_rocks_and(&req, |package: &LocalPackage| {
                    callback.call(package.clone()).unwrap_or(false)
                })
                .into_lua_err()
            },
        );
        methods.add_method("rock_layout", |_, this, package: LocalPackage| {
            this.installed_rock_layout(&package)
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
        methods.add_method("rock", |_, this, package: LocalPackage| {
            this.dependency(&package).into_lua_err()
        });
        methods.add_method("lockfile", |_, this, ()| this.lockfile().into_lua_err());
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

impl IntoLua for RockMatches {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;
        let is_found = self.is_found();

        table.set("is_found", lua.create_function(move |_, ()| Ok(is_found))?)?;

        match self {
            RockMatches::NotFound(package_req) => table.set("not_found", package_req)?,
            RockMatches::Single(local_package_id) => table.set("single", local_package_id)?,
            RockMatches::Many(local_package_ids) => {
                table.set("many", local_package_ids.into_iter().collect_vec())?
            }
        }

        Ok(mlua::Value::Table(table))
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::PathCopy;
    use itertools::Itertools;
    use std::path::PathBuf;

    use insta::assert_yaml_snapshot;

    use crate::{
        config::{ConfigBuilder, LuaVersion},
        lockfile::{LocalPackage, LocalPackageHashes, LockConstraint},
        package::{PackageName, PackageSpec, PackageVersion},
        remote_package_source::RemotePackageSource,
        rockspec::RockBinaries,
        tree::RockLayout,
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

        let neorg = tree.dependency(&package).unwrap();

        assert_eq!(
            neorg,
            RockLayout {
                bin: tree_path.join("5.1/bin"),
                rock_path: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1")),
                etc: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc")),
                lib: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/lib")),
                src: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/src")),
                conf: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc/conf")),
                doc: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/etc/doc")),
            }
        );

        let package = LocalPackage::from(
            &PackageSpec::parse("lua-cjson".into(), "2.1.0-1".into()).unwrap(),
            LockConstraint::Unconstrained,
            RockBinaries::default(),
            RemotePackageSource::Test,
            None,
            mock_hashes.clone(),
        );

        let id = package.id();

        let lua_cjson = tree.dependency(&package).unwrap();

        assert_eq!(
            lua_cjson,
            RockLayout {
                bin: tree_path.join("5.1/bin"),
                rock_path: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1")),
                etc: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1/etc")),
                lib: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1/lib")),
                src: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1/src")),
                conf: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1/etc/conf")),
                doc: tree_path.join(format!("5.1/{id}-lua-cjson@2.1.0-1/etc/doc")),
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

        let neorg = tree
            .dependency(&LocalPackage::from(
                &PackageSpec::parse("neorg".into(), "8.0.0-1-1".into()).unwrap(),
                LockConstraint::Unconstrained,
                RockBinaries::default(),
                RemotePackageSource::Test,
                None,
                mock_hashes.clone(),
            ))
            .unwrap();
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
            .map(|var| variables::substitute(&[&neorg], var))
            .try_collect()
            .unwrap();
        assert_eq!(
            result,
            vec![
                neorg.rock_path.to_string_lossy().to_string(),
                neorg.lib.to_string_lossy().to_string(),
                neorg.src.to_string_lossy().to_string(),
                neorg.bin.to_string_lossy().to_string(),
                neorg.conf.to_string_lossy().to_string(),
                neorg.doc.to_string_lossy().to_string(),
            ]
        );
    }
}
