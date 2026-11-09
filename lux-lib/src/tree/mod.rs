use crate::package;
use crate::{
    config::Config,
    fs,
    lockfile::{self, LockedPackage, LockedPackageId, LockedPackageSpec, Lockfile, ReadOnly},
    lua, path,
    rockspec::RockBinaries,
    store::{Entry, Store},
    transaction::Transaction,
    variables::{GetVariableError, HasVariables},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use itertools::Itertools;
use miette::Diagnostic;
use nonempty::NonEmpty;
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;
mod dist;
mod layout;

pub use dist::*;
pub use layout::*;

const LOCKFILE_NAME: &str = "lux.lock";

/// A tree is a collection of files where installed rocks are located.
///
/// `lux` diverges from the traditional hierarchy employed by luarocks.
/// Instead, we opt for a much simpler approach:
///
/// - /<lua-version> - contains rocks
/// - /<lua-version>/<rock>/etc - documentation and supplementary files for the rock
/// - /<lua-version>/<rock>/lib - shared libraries (.so files)
/// - /<lua-version>/<rock>/src - library code for the rock
/// - /bin - binary files produced by various rocks
pub trait InstallView: Send + Sync {
    /// The concrete slot this target installs into (read-only path view).
    fn slot(&self) -> &Slot;
    /// The slot used to build packages that are not installed yet.
    fn build_slot(&self) -> Result<Slot, Error>;
    /// A transaction that installs build dependencies for this target.
    fn build_transaction(&self) -> Result<Transaction<'_>, Error>;

    /// The Lua version for which to install packages.
    fn version(&self) -> &lua::Version {
        self.slot().version()
    }
    /// The root directory of the tree
    fn root(&self) -> PathBuf {
        self.slot().root()
    }
    /// Where wrapped package binaries are installed
    fn bin(&self) -> PathBuf {
        self.slot().bin()
    }
    /// Where unwrapped package binaries are installed
    fn unwrapped_bin(&self) -> PathBuf {
        self.slot().unwrapped_bin()
    }
    /// The standard install layout for a package.
    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        self.slot().layout_for(package)
    }
    /// Create a [`Lockfile`] for this tree.
    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        self.slot().lockfile()
    }
    /// Get this tree's lockfile path.
    fn lockfile_path(&self) -> PathBuf {
        self.slot().lockfile_path()
    }
    /// List the packages that are installed in this tree.
    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        self.slot().list()
    }
    /// Find installed rocks that match the given [`package::Req`].
    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
        self.slot().match_rocks(req)
    }
    /// Whether the package's install layout exists.
    fn is_installed(&self, package: &LockedPackage) -> bool {
        self.layout_for(package.spec()).root.is_dir()
    }
}

/// An [`InstallView`] that can install packages, potentially incrementally.
pub trait InstallSession: InstallView {
    /// Create the standard directories (src, lib, etc.) for a package.
    fn prepare(&self, package: &LockedPackage) -> Result<(), Error>;
    /// Reuse an already-complete store entry for `package`, if one exists.
    fn reuse_complete(
        &self,
        _package: &LockedPackage,
        _entry_type: EntryType,
    ) -> Result<Option<LockedPackage>, Error> {
        Ok(None)
    }
    /// Apply the custom layout once a package has been fully installed.
    ///
    /// For entrypoints, this creates the symlinks for the custom layout, if one is configured.
    /// Must be called after the package's files have been written to the standard layout.
    fn finalize(&self, _package: &LockedPackage, _entry_type: EntryType) -> Result<(), Error> {
        Ok(())
    }
    /// Remove the install layout directories.
    ///
    /// For entrypoints, this also applies the custom layout, if one is configured.
    fn cleanup(&self, _package: &LockedPackage, _entry_type: EntryType) -> Result<(), Error> {
        Err(Error::Other(
            "package cleanup is not supported for this install target".into(),
        ))
    }
}

/// Marker trait for [`InstallSession`]s whose packages can be uninstalled.
pub trait UninstallTarget: InstallSession {}

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
    /// Create the standard install layout rooted at `root`.
    pub fn new(root: PathBuf) -> Self {
        let etc = root.join("etc");
        let lib = root.join("lib");
        let src = root.join("src");
        let bin = root.join("bin");
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

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] lockfile::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] lockfile::FlushLockfileError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("existing tree at '{}' uses an unsupported layout", path.display())]
    #[diagnostic(help("move or remove the existing tree directory, then retry"))]
    LegacyTree { path: PathBuf },
    #[error("{0}")]
    Other(String),
}

impl HasVariables for RockLayout {
    fn get_variable(&self, var: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match var {
            "PREFIX" => Some(path::quoted(&self.root)),
            "LIBDIR" => Some(path::quoted(&self.lib)),
            "LUADIR" => Some(path::quoted(&self.src)),
            "BINDIR" => Some(path::quoted(&self.bin)),
            "CONFDIR" => Some(path::quoted(&self.conf)),
            "DOCDIR" => Some(path::quoted(&self.doc)),
            _ => None,
        })
    }
}

impl Tree {
    /// NOTE: This is exposed for use by the config module.
    /// Use `Config::user_tree()`
    pub(crate) fn new(
        root: PathBuf,
        version: lua::Version,
        config: &Config,
    ) -> Result<Self, Error> {
        fs::sync::create_dir_all(&root)?;
        fs::sync::write(root.join(".gitignore"), "*")?;

        let store = Store::new(root.join("store").join(version.to_string()));
        let regular = open_slot(&root, &version, Kind::Regular)?;

        Ok(Self {
            root,
            version,
            store,
            entrypoint_layout: config.entrypoint_layout().cloned(),
            regular,
        })
    }

    pub(crate) fn new_flat(
        root: PathBuf,
        version: lua::Version,
        config: &Config,
    ) -> Result<Self, Error> {
        fs::sync::create_dir_all(&root)?;
        fs::sync::write(root.join(".gitignore"), "*")?;
        let root = root.join(version.to_string());
        fs::sync::create_dir_all(root.join("bin"))?;
        let store = Store::new(root.join("store").join(version.to_string()));

        Ok(Self {
            regular: Slot {
                root: root.clone(),
                version: version.clone(),
            },
            root,
            version,
            store,
            entrypoint_layout: config.entrypoint_layout().cloned(),
        })
    }

    pub fn regular(&self) -> &Slot {
        &self.regular
    }

    pub fn build(&self) -> Result<Slot, Error> {
        open_slot(&self.root, &self.version, Kind::Build)
    }

    pub fn test(&self) -> Result<Slot, Error> {
        open_slot(&self.root, &self.version, Kind::Test)
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn entrypoint_layout(&self) -> Option<&Arc<dyn CustomRockLayout>> {
        self.entrypoint_layout.as_ref()
    }

    pub(crate) fn build_base(&self) -> PathBuf {
        Kind::Build.base_path(&self.root)
    }

    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn version(&self) -> &lua::Version {
        &self.version
    }

    pub(crate) fn prune_store(&self, referenced: &HashSet<PathBuf>) -> Result<(), Error> {
        self.store.prune(referenced)
    }
}

impl InstallView for Tree {
    fn slot(&self) -> &Slot {
        self.regular()
    }

    fn build_slot(&self) -> Result<Slot, Error> {
        self.build()
    }

    fn build_transaction(&self) -> Result<Transaction<'_>, Error> {
        let slot = self.build()?;
        Transaction::new(self, &slot)
    }
}

fn open_slot(tree_root: &Path, version: &lua::Version, kind: Kind) -> Result<Slot, Error> {
    let base = kind.base_path(tree_root);
    let root = kind.versioned_path(tree_root, version);
    ensure_tree_dir(&base, version, kind, &root)?;
    Ok(Slot {
        root,
        version: version.clone(),
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Kind {
    Regular,
    Build,
    Test,
}

impl Kind {
    fn base_path(self, tree_root: &Path) -> PathBuf {
        match self {
            Self::Regular => tree_root.join("dependencies"),
            Self::Build => tree_root.join("build_dependencies"),
            Self::Test => tree_root.join("test_dependencies"),
        }
    }

    fn versioned_path(self, tree_root: &Path, version: &lua::Version) -> PathBuf {
        self.base_path(tree_root).join(version.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct Tree {
    root: PathBuf,
    version: lua::Version,
    store: Store,
    entrypoint_layout: Option<Arc<dyn CustomRockLayout>>,
    regular: Slot,
}

#[derive(Clone, Debug)]
pub struct Slot {
    root: PathBuf,
    version: lua::Version,
}

/// A temporary copy of a slot. Dropping it without committing removes the copy.
#[derive(Debug)]
pub(crate) struct StagingTree {
    pointer: PathBuf,
    slot: Slot,
    staging: tempfile::TempDir,
}

impl StagingTree {
    pub(crate) fn new(source: &Slot) -> Result<Self, Error> {
        let base = source.root.parent().ok_or_else(|| {
            Error::Other(format!(
                "tree slot '{}' has no parent",
                source.root.display()
            ))
        })?;
        let source_root = fs::sync::canonicalize(&source.root)?;
        let staging = tempfile::Builder::new()
            .prefix(&format!(".tree-staging-{}", source.version))
            .tempdir_in(base)
            .map_err(|source| fs::Error::CreateTempDir { source })?;
        fs::sync::copy_tree(&source_root, staging.path())?;
        fs::sync::create_dir_all(staging.path().join("bin"))?;

        Ok(Self {
            pointer: source.root.clone(),
            slot: Slot {
                root: staging.path().to_path_buf(),
                version: source.version.clone(),
            },
            staging,
        })
    }

    pub(crate) fn slot(&self) -> &Slot {
        &self.slot
    }

    pub(crate) fn publish(
        &self,
        tree: &Tree,
        package: &LockedPackage,
        entry: Option<&Entry>,
        entry_type: EntryType,
    ) -> Result<(), Error> {
        if let Some(entry) = entry {
            self.link_entry(entry)?;
        }

        let binaries = self.finalize_view(tree, package, entry_type)?;
        let lockfile = self.slot.lockfile()?;
        if lockfile.binaries(&package.id()).is_none() {
            lockfile.write_guard().set_binaries(package, binaries);
        }

        Ok(())
    }

    fn link_entry(&self, entry: &Entry) -> Result<(), Error> {
        let target = self.slot.layout_for(entry.package()).root;
        if target
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            fs::sync::remove_dir_all(&target)?;
        }
        fs::sync::link_dir(entry.path(), &target)?;
        Ok(())
    }

    fn finalize_view(
        &self,
        tree: &Tree,
        package: &LockedPackage,
        entry_type: EntryType,
    ) -> Result<RockBinaries, Error> {
        let layout = self.slot.layout_for(&package.spec);
        let binaries = link_binaries(&layout.bin, &self.slot.bin())?;

        if let Some(custom_layout) = tree.entrypoint_layout() {
            if entry_type.is_entrypoint() || custom_layout.layout_dependencies() {
                custom_layout.apply_layout(tree, &self.slot, &package.spec)?;
            }
        }

        Ok(binaries)
    }

    pub(crate) fn commit(&mut self) -> Result<(), Error> {
        let previous = std::fs::read_link(&self.pointer).ok();
        if let Err(error) = fs::sync::replace_symlink(&self.slot.root, &self.pointer) {
            self.abort();
            return Err(error.into());
        }
        self.staging.disable_cleanup(true);

        if let Some(previous) = previous {
            let base = self.pointer.parent().unwrap_or(Path::new("."));
            let previous = if previous.is_absolute() {
                previous
            } else {
                base.join(previous)
            };
            if previous != self.slot.root && slot_name_is_staging(&previous) {
                let _ = fs::sync::remove_dir_all(previous);
            }
        }

        Ok(())
    }

    pub(crate) fn abort(&mut self) {
        let _ = fs::sync::remove_dir_all(self.staging.path());
        self.staging.disable_cleanup(true);
    }
}

fn ensure_tree_dir(
    base: &Path,
    version: &lua::Version,
    kind: Kind,
    pointer: &Path,
) -> Result<(), Error> {
    fs::sync::create_dir_all(base)?;

    let linked_to_dir = match pointer.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => pointer.is_dir(),
        Ok(_) => {
            return Err(Error::LegacyTree {
                path: pointer.to_path_buf(),
            })
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(fs::Error::Metadata {
                path: pointer.to_path_buf(),
                source,
            }
            .into())
        }
    };

    if !linked_to_dir {
        create_empty(base, version, kind, pointer)?;
    }

    fs::sync::create_dir_all(pointer.join("bin"))?;
    Ok(())
}

fn create_empty(
    base: &Path,
    version: &lua::Version,
    kind: Kind,
    pointer: &Path,
) -> Result<(), Error> {
    let root = allocate_tree_dir(base, format!("{kind:?}:{version}:empty"))?;
    fs::sync::replace_symlink(&root, pointer)?;
    Ok(())
}

impl Slot {
    pub fn version(&self) -> &lua::Version {
        &self.version
    }

    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn unwrapped_bin(&self) -> PathBuf {
        self.bin().join("unwrapped")
    }

    pub fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        Ok(Lockfile::new(self.lockfile_path())?)
    }

    pub fn lockfile_path(&self) -> PathBuf {
        self.root.join(LOCKFILE_NAME)
    }

    pub fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        Ok(self.lockfile()?.list())
    }

    pub fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
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

    pub fn match_rocks_and<F>(&self, req: &package::Req, filter: F) -> Result<RockMatches, Error>
    where
        F: Fn(&LockedPackage) -> bool,
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

    pub fn as_rock_list(&self) -> Result<Vec<LockedPackage>, Error> {
        let rock_list = self.list()?;

        Ok(rock_list.values().flatten().cloned().collect())
    }

    pub fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        RockLayout::new(self.root.join(format!(
            "{}-{}@{}",
            package.id(),
            package.name(),
            package.version()
        )))
    }

    pub fn is_installed(&self, package: &LockedPackage) -> bool {
        self.layout_for(package.spec()).root.is_dir()
    }
}

fn allocate_tree_dir(base: &Path, seed: impl AsRef<[u8]>) -> Result<PathBuf, Error> {
    let hash = hex::encode(Sha256::digest(seed.as_ref()));
    for suffix in 0.. {
        let name = if suffix == 0 {
            format!(".tree-{hash}")
        } else {
            format!(".tree-{hash}-{suffix}")
        };
        let path = base.join(name);
        match fs::sync::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(fs::Error::CreateDir { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }

    unreachable!()
}

fn slot_name_is_staging(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(".tree-"))
}

pub(crate) fn link_binaries(src: &Path, dest: &Path) -> Result<RockBinaries, Error> {
    let mut binaries = RockBinaries::default();
    if !src.is_dir() {
        return Ok(binaries);
    }

    let entries = WalkDir::new(src).into_iter().filter_entry(|entry| {
        !(entry.depth() == 1 && entry.file_name() == OsStr::new("unwrapped"))
    });

    for entry in entries {
        let entry = entry.map_err(|error| Error::Other(error.to_string()))?;
        if entry.file_type().is_dir() {
            continue;
        }

        let path = entry.path();
        let relative = path.strip_prefix(src).map_err(|_| {
            Error::Other(format!(
                "walked path '{}' is not under '{}'",
                path.display(),
                src.display()
            ))
        })?;
        let relative = relative.to_path_buf();
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        fs::sync::link_file(path, dest.join(&relative))?;
        binaries.push(relative);
    }

    Ok(binaries)
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
    NotFound(package::Req),
    Single(LockedPackageId),
    Many(NonEmpty<LockedPackageId>),
}

// Loosely mimic the Option<T> functions.
impl RockMatches {
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Single(_) | Self::Many(_))
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::{PathChild, PathCopy};
    use itertools::Itertools;
    use std::path::PathBuf;

    use crate::package;
    use crate::remote;

    use insta::assert_yaml_snapshot;

    use crate::{
        config, fs,
        lockfile::{self, LockedPackage, LockedPackageHashes},
        lua,
        transaction::Transaction,
        tree::{self, InstallSession, InstallView, RockLayout, Slot, Tree},
        variables,
    };

    fn sample_tree(temp: &assert_fs::TempDir) -> (Tree, Slot) {
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let slot = tree.regular().clone();
        temp.child("dependencies/5.1")
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-tree/dependencies/5.1"),
                &["**"],
            )
            .unwrap();
        (tree, slot)
    }

    #[test]
    fn is_installed_requires_the_layout_directory() {
        let temp = assert_fs::TempDir::new().unwrap();
        let (_tree_ctx, tree) = sample_tree(&temp);
        let package = tree
            .list()
            .unwrap()
            .into_values()
            .flatten()
            .find(|package| package.name().to_string() == "neorg")
            .unwrap();

        assert!(tree.is_installed(&package));

        fs::sync::remove_dir_all(tree.layout_for(package.spec()).root).unwrap();
        assert!(!tree.is_installed(&package));
    }

    #[test]
    fn rock_layout() {
        let temp = assert_fs::TempDir::new().unwrap();
        let (_tree_ctx, tree) = sample_tree(&temp);
        let tree_path = tree.root();

        let mock_hashes = LockedPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        };

        let package = LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            mock_hashes.clone(),
        );

        let id = package.id();

        assert_eq!(
            tree.layout_for(&package.spec),
            RockLayout {
                bin: tree_path.join(format!("{id}-neorg@8.0.0-1/bin")),
                root: tree_path.join(format!("{id}-neorg@8.0.0-1")),
                etc: tree_path.join(format!("{id}-neorg@8.0.0-1/etc")),
                lib: tree_path.join(format!("{id}-neorg@8.0.0-1/lib")),
                src: tree_path.join(format!("{id}-neorg@8.0.0-1/src")),
                conf: tree_path.join(format!("{id}-neorg@8.0.0-1/etc/conf")),
                doc: tree_path.join(format!("{id}-neorg@8.0.0-1/etc/doc")),
            }
        );
    }

    fn test_package() -> LockedPackage {
        LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            LockedPackageHashes {
                rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
                source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
            },
        )
    }

    #[test]
    fn staging_abort_rolls_back_without_touching_the_tree() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let pointer = regular.root();
        let previous = std::fs::read_link(&pointer).unwrap();
        fs::sync::write(regular.root().join("stable-marker"), "stable").unwrap();

        let package = test_package();
        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        let staging_root = transaction.root();
        fs::sync::write(staging_root.join("staged-marker"), "staged").unwrap();
        transaction
            .lockfile()
            .unwrap()
            .write_guard()
            .add_entrypoint(&package);
        drop(transaction);

        assert_eq!(std::fs::read_link(&pointer).unwrap(), previous);
        assert!(pointer.join("stable-marker").is_file());
        assert!(!staging_root.exists());
        assert!(regular.lockfile().unwrap().get(&package.id()).is_none());
        assert!(pointer.parent().unwrap().join(previous).exists());
    }

    #[test]
    fn staging_commit_replaces_pointer() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let pointer = regular.root();
        let previous = std::fs::read_link(&pointer).unwrap();

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        fs::sync::write(transaction.root().join("marker"), "marker").unwrap();
        transaction.commit().unwrap();

        let current = std::fs::read_link(&pointer).unwrap();
        let current = pointer.parent().unwrap().join(current);
        assert!(current.join("marker").is_file());
        assert!(!pointer.parent().unwrap().join(previous).exists());
    }

    #[test]
    fn build_tree_reuses_the_build_slot() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let build_tree = tree.build().unwrap();
        let nested = tree.build().unwrap();
        assert_eq!(build_tree.root(), nested.root());
        assert!(build_tree.root().ends_with("build_dependencies/5.1"));
    }

    #[test]
    fn legacy_tree_layout_is_rejected_without_deleting_it() {
        let temp = assert_fs::TempDir::new().unwrap();
        let legacy = temp.path().join("dependencies/5.1");
        fs::sync::create_dir_all(&legacy).unwrap();
        fs::sync::write(legacy.join("lux.lock"), "legacy").unwrap();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let error = config.user_tree(lua::Version::Lua51).unwrap_err();

        assert!(matches!(error, tree::Error::LegacyTree { .. }));
        assert!(legacy.join("lux.lock").is_file());
    }

    #[test]
    fn dangling_pointer_is_replaced_with_an_empty_tree() {
        let temp = assert_fs::TempDir::new().unwrap();
        let pointer = temp.path().join("dependencies/5.1");
        fs::sync::create_dir_all(pointer.parent().unwrap()).unwrap();
        fs::sync::replace_symlink(temp.path().join("missing"), &pointer).unwrap();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();

        assert_eq!(regular.root(), pointer);
        assert!(pointer.is_dir());
        assert!(!temp.path().join("missing").exists());
    }

    #[test]
    fn regular_tree_lives_under_dependencies() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

        assert_eq!(tree.regular().root(), temp.path().join("dependencies/5.1"));
        assert_eq!(
            tree.build().unwrap().root(),
            temp.path().join("build_dependencies/5.1")
        );
        assert_eq!(
            tree.test().unwrap().root(),
            temp.path().join("test_dependencies/5.1")
        );

        let regular = tree.regular();
        let transaction = Transaction::new(&tree, &regular).unwrap();
        assert_eq!(
            transaction.root().parent().unwrap(),
            temp.path().join("dependencies")
        );
    }

    #[test]
    fn staged_binaries_link_to_the_store() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let package = LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            LockedPackageHashes {
                rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
                source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
            },
        );

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        transaction
            .finalize(&package, tree::EntryType::Entrypoint)
            .unwrap();

        let build_id = crate::build::Id::from_package(&package);
        assert!(temp
            .path()
            .join("store/5.1")
            .join(format!("{build_id}-neorg@8.0.0-1"))
            .is_dir());

        transaction.commit().unwrap();

        let binary = regular.bin().join("tool");
        assert!(binary.is_file());
        let resolved = std::fs::canonicalize(binary).unwrap();
        assert!(resolved.starts_with(std::fs::canonicalize(temp.path()).unwrap().join("store")));
    }

    #[test]
    fn commit_with_entrypoint_records_the_package_and_keeps_binaries() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let package = test_package();

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.rockspec_path(), "return {}").unwrap();
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        transaction
            .finalize(&package, tree::EntryType::Entrypoint)
            .unwrap();

        transaction.commit_with_entrypoint(&package).unwrap();

        let lockfile = regular.lockfile().unwrap();
        assert!(lockfile.get(&package.id()).is_some());
        assert!(lockfile.binaries(&package.id()).is_some());
        assert!(regular.bin().join("tool").is_file());
    }

    #[test]
    fn reuse_complete_relinks_without_touching_the_store() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let package = LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            LockedPackageHashes {
                rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
                source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
            },
        );

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.rockspec_path(), "return {}").unwrap();
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        transaction
            .finalize(&package, tree::EntryType::Entrypoint)
            .unwrap();
        transaction
            .lockfile()
            .unwrap()
            .write_guard()
            .add_entrypoint(&package);
        transaction.commit().unwrap();

        let store_root = std::fs::canonicalize(regular.layout_for(&package.spec).root).unwrap();
        fs::sync::write(store_root.join("built"), "original").unwrap();

        let reuse = Transaction::new(&tree_ctx, &regular).unwrap();
        let reused = reuse
            .reuse_complete(&package, tree::EntryType::Entrypoint)
            .unwrap();
        assert!(reused.is_some());
        assert_eq!(
            fs::sync::read_to_string(store_root.join("built")).unwrap(),
            "original"
        );
        assert!(reuse.bin().join("tool").is_file());
        reuse.commit().unwrap();
    }

    #[test]
    fn reuse_complete_regression_republishes_when_package_is_not_in_lockfile() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let package = test_package();

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.rockspec_path(), "return {}").unwrap();
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        transaction
            .finalize(&package, tree::EntryType::Entrypoint)
            .unwrap();
        transaction
            .lockfile()
            .unwrap()
            .write_guard()
            .remove_by_id(&package.id());
        transaction.commit().unwrap();

        assert!(regular.lockfile().unwrap().get(&package.id()).is_none());
        assert!(regular
            .lockfile()
            .unwrap()
            .binaries(&package.id())
            .is_none());

        let store_root = std::fs::canonicalize(regular.layout_for(&package.spec).root).unwrap();
        fs::sync::write(store_root.join("built"), "original").unwrap();

        let reuse = Transaction::new(&tree_ctx, &regular).unwrap();
        let reused = reuse
            .reuse_complete(&package, tree::EntryType::Entrypoint)
            .unwrap();
        assert!(reused.is_some());
        assert_eq!(
            fs::sync::read_to_string(store_root.join("built")).unwrap(),
            "original"
        );
        assert!(reuse.bin().join("tool").is_file());
        assert!(reuse.lockfile().unwrap().binaries(&package.id()).is_some());
        reuse.commit().unwrap();
        assert!(regular.bin().join("tool").is_file());
    }

    #[test]
    fn cleanup_removes_wrapped_and_unwrapped_binaries_but_keeps_store_entry() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree_ctx = config.user_tree(lua::Version::Lua51).unwrap();
        let regular = tree_ctx.regular();
        let package = test_package();

        let transaction = Transaction::new(&tree_ctx, &regular).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.rockspec_path(), "return {}").unwrap();
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        let mut binaries = crate::rockspec::RockBinaries::default();
        binaries.push(PathBuf::from("tool"));
        binaries.push(PathBuf::from("../escape"));
        transaction
            .lockfile()
            .unwrap()
            .write_guard()
            .set_binaries(&package, binaries);
        transaction
            .finalize(&package, tree::EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let store_root = std::fs::canonicalize(regular.layout_for(&package.spec).root).unwrap();
        assert!(regular.bin().join("tool").is_file());

        let cleanup = Transaction::new(&tree_ctx, &regular).unwrap();
        fs::sync::create_dir_all(cleanup.unwrapped_bin()).unwrap();
        fs::sync::write(cleanup.unwrapped_bin().join("tool"), "tool").unwrap();
        let escaped = cleanup.root().join("escape");
        fs::sync::write(&escaped, "escape").unwrap();
        cleanup
            .cleanup(&package, tree::EntryType::Entrypoint)
            .unwrap();
        assert!(!cleanup.bin().join("tool").exists());
        assert!(!cleanup.unwrapped_bin().join("tool").exists());
        assert!(escaped.is_file());
        assert!(!cleanup.layout_for(&package.spec).root.exists());
        cleanup.commit().unwrap();

        assert!(!regular.bin().join("tool").exists());
        assert!(!regular.unwrapped_bin().join("tool").exists());
        assert!(store_root.is_dir());
        assert!(store_root.join("package.rockspec").is_file());
    }

    #[test]
    fn tree_list() {
        let temp = assert_fs::TempDir::new().unwrap();
        let (_tree_ctx, tree) = sample_tree(&temp);
        let result = tree.list().unwrap();
        // note: sorted_redaction doesn't work because we have a nested Vec
        let sorted_result: Vec<(package::Name, Vec<package::Version>)> = result
            .into_iter()
            .sorted()
            .map(|(name, package)| {
                (
                    name,
                    package
                        .into_iter()
                        .map(|package| package.version().clone())
                        .sorted()
                        .collect_vec(),
                )
            })
            .collect_vec();

        assert_yaml_snapshot!(sorted_result)
    }

    #[test]
    fn rock_layout_substitute() {
        let temp = assert_fs::TempDir::new().unwrap();
        let (_tree_ctx, tree) = sample_tree(&temp);

        let mock_hashes = LockedPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        };

        let package = LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            mock_hashes.clone(),
        );
        let layout = tree.layout_for(&package.spec);
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
