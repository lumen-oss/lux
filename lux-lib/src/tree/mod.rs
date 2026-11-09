use crate::package;
use crate::store::{Entry, Store};
use crate::{
    config::Config,
    fs,
    lockfile::{self, LockedPackage, LockedPackageId, LockedPackageSpec, Lockfile, ReadOnly},
    lua, path,
    variables::{GetVariableError, HasVariables},
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use itertools::Itertools;
use miette::Diagnostic;
use nonempty::NonEmpty;
use sha2::{Digest, Sha256};
use thiserror::Error;
mod dist;
pub mod install;
mod layout;
mod list;
pub mod staging;

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
pub trait InstallTree: Send + Sync {
    /// The Lua version for which to install packages.
    fn version(&self) -> &lua::Version;
    /// The root directory of the tree
    fn root(&self) -> PathBuf;
    /// Where wrapped package binaries are installed
    fn bin(&self) -> PathBuf;
    /// Where unwrapped package binaries are installed
    fn unwrapped_bin(&self) -> PathBuf;
    /// The standard install layout for a package.
    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout;
    /// Create a [`Lockfile`] for this tree.
    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error>;
    /// Get this tree's lockfile path.
    fn lockfile_path(&self) -> PathBuf;
    /// The tree in which to install build dependencies.
    fn build_tree(&self, config: &Config) -> Result<Tree, Error>;
    /// The tree in which to install test dependencies.
    fn test_tree(&self, config: &Config) -> Result<Tree, Error>;
    /// List the packages that are installed in this tree.
    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error>;
    /// Find installed rocks that match the given [`package::Req`].
    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error>;
    /// Create the standard directories (src, lib, etc.) for a package.
    fn prepare(&self, package: &LockedPackage) -> Result<(), Error>;
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
    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error>;
}

pub trait Stageable: InstallTree {
    fn stage(&self) -> Result<StagingTree, Error>;
    fn commit_stage(&self, staging: &StagingTree) -> Result<(), Error>;
    fn abort_stage(&self, staging: &StagingTree);
}

pub trait Installable: InstallTree {}

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
    /// Use `Config::tree()`
    pub(crate) fn new(
        root: PathBuf,
        version: lua::Version,
        config: &Config,
    ) -> Result<Self, Error> {
        fs::sync::create_dir_all(&root)?;
        fs::sync::write(root.join(".gitignore"), "*")?;

        let version_dir = root.join(version.to_string());
        if version_dir.is_dir() && !version_dir.is_symlink() {
            move_legacy_tree(
                &version_dir.join("build_dependencies"),
                &TreeKind::Build.base(&root),
                &version,
            )?;
            move_legacy_tree(
                &version_dir.join("test_dependencies"),
                &TreeKind::Test.base(&root),
                &version,
            )?;
        }

        let store = Arc::new(Store::new(root.join("store").join(version.to_string())));
        Self::open(store, version, root, TreeKind::Regular, config)
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
        let store = Arc::new(Store::new(root.join("store").join(version.to_string())));
        Ok(Self {
            slot: TreeSlot {
                version,
                tree_root: root.clone(),
                root,
                base: PathBuf::new(),
                kind: TreeKind::Regular,
                store,
                entrypoint_layout: config.entrypoint_layout().cloned(),
                pending: Arc::new(Mutex::new(HashMap::new())),
            },
        })
    }

    fn open(
        store: Arc<Store>,
        version: lua::Version,
        tree_root: PathBuf,
        kind: TreeKind,
        config: &Config,
    ) -> Result<Self, Error> {
        Ok(Self {
            slot: TreeSlot::open(store, version, tree_root, kind, config)?,
        })
    }

    pub(crate) fn from_slot(slot: TreeSlot) -> Self {
        Self { slot }
    }

    /// The currently active tree view.
    pub fn current(&self) -> &TreeSlot {
        &self.slot
    }

    fn staging(&self) -> Result<StagingTree, Error> {
        self.slot.staging()
    }

    fn commit(&self, staging: &StagingTree) -> Result<(), Error> {
        let slot = &staging.slot;
        if slot.base != self.slot.base
            || slot.kind != self.slot.kind
            || slot.version != self.slot.version
        {
            return Err(Error::Other(
                "cannot commit a slot into a different tree".into(),
            ));
        }

        let previous = std::fs::read_link(&self.slot.root).ok();
        if let Err(error) = fs::sync::replace_symlink(&slot.root, &self.slot.root) {
            self.abort(staging);
            return Err(error.into());
        }

        if let Some(previous) = previous {
            let previous = self.slot.base.join(previous);
            if previous != slot.root && slot_name_is_staging(&previous) {
                let _ = fs::sync::remove_dir_all(previous);
            }
        }

        Ok(())
    }

    fn abort(&self, staging: &StagingTree) {
        let slot = &staging.slot;
        if slot.base == self.slot.base
            && slot.kind == self.slot.kind
            && slot.version == self.slot.version
            && slot_name_is_staging(&slot.root)
        {
            let _ = fs::sync::remove_dir_all(&slot.root);
        }
    }

    pub fn match_rocks_and<F>(&self, req: &package::Req, filter: F) -> Result<RockMatches, Error>
    where
        F: Fn(&LockedPackage) -> bool,
    {
        self.slot.match_rocks_and(req, filter)
    }

    pub(crate) fn build_tree_base(&self) -> PathBuf {
        TreeKind::Build.base(&self.slot.tree_root)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TreeKind {
    Regular,
    Build,
    Test,
}

impl TreeKind {
    fn base(self, tree_root: &Path) -> PathBuf {
        match self {
            Self::Regular => tree_root.to_path_buf(),
            Self::Build => tree_root.join("build_dependencies"),
            Self::Test => tree_root.join("test_dependencies"),
        }
    }

    fn pointer(self, tree_root: &Path, version: &lua::Version) -> PathBuf {
        self.base(tree_root).join(version.to_string())
    }
}

/// A stable handle to an install tree.
///
/// The tree's root is a symlink to a concrete [`TreeSlot`]. Installing into a
/// staging slot and swapping the pointer makes a set of changes atomic.
#[derive(Clone, Debug)]
pub struct Tree {
    slot: TreeSlot,
}

/// A concrete tree view. All filesystem writes for a tree target one of these.
#[derive(Clone, Debug)]
pub struct TreeSlot {
    version: lua::Version,
    tree_root: PathBuf,
    root: PathBuf,
    base: PathBuf,
    kind: TreeKind,
    store: Arc<Store>,
    entrypoint_layout: Option<Arc<dyn CustomRockLayout>>,
    pending: Arc<Mutex<HashMap<LockedPackageId, Entry>>>,
}

/// A transactional tree view that can be committed into a [`Tree`].
#[derive(Clone, Debug)]
pub struct StagingTree {
    slot: TreeSlot,
}

impl TreeSlot {
    fn open(
        store: Arc<Store>,
        version: lua::Version,
        tree_root: PathBuf,
        kind: TreeKind,
        config: &Config,
    ) -> Result<Self, Error> {
        let base = kind.base(&tree_root);
        fs::sync::create_dir_all(&base)?;
        let root = kind.pointer(&tree_root, &version);

        match root.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() && root.is_dir() => {}
            Ok(metadata) if metadata.is_dir() => {
                let legacy = available_tree_dir(&base, format!("legacy:{kind:?}:{version}"));
                fs::sync::rename(&root, &legacy)?;
                fs::sync::replace_symlink(&legacy, &root)?;
            }
            Ok(_) => {
                fs::sync::remove_path(&root)?;
                Self::create_empty(&base, &version, kind, &root)?;
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Self::create_empty(&base, &version, kind, &root)?;
            }
            Err(source) => {
                return Err(fs::Error::Metadata { path: root, source }.into());
            }
        }

        fs::sync::create_dir_all(root.join("bin"))?;
        Ok(Self {
            version,
            tree_root,
            root,
            base,
            kind,
            store,
            entrypoint_layout: config.entrypoint_layout().cloned(),
            pending: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn create_empty(
        base: &Path,
        version: &lua::Version,
        kind: TreeKind,
        pointer: &Path,
    ) -> Result<(), Error> {
        let root = allocate_tree_dir(base, format!("{kind:?}:{version}:empty"))?;
        fs::sync::create_dir_all(root.join("bin"))?;
        fs::sync::replace_symlink(&root, pointer)?;
        Ok(())
    }

    fn staging(&self) -> Result<StagingTree, Error> {
        let mut seed = format!("{:?}:{}:", self.kind, self.version).into_bytes();
        if let Ok(lockfile) = fs::sync::read_to_string(self.lockfile_path()) {
            seed.extend(lockfile.bytes());
        }
        let root = allocate_tree_dir(&self.base, seed)?;
        fs::sync::create_dir_all(root.join("bin"))?;

        let slot = Self {
            version: self.version.clone(),
            tree_root: self.tree_root.clone(),
            root: root.clone(),
            base: self.base.clone(),
            kind: self.kind,
            store: self.store.clone(),
            entrypoint_layout: self.entrypoint_layout.clone(),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };

        let lockfile_path = self.lockfile_path();
        if lockfile_path.is_file() {
            fs::sync::copy(&lockfile_path, slot.lockfile_path())?;
        }

        let lockfile = self.lockfile()?;
        for package in lockfile.rocks().values() {
            let current = self.view_layout_for(&package.spec).root;
            let target = slot.view_layout_for(&package.spec).root;

            match current.symlink_metadata() {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let link =
                        std::fs::read_link(&current).map_err(|source| fs::Error::Metadata {
                            path: current.clone(),
                            source,
                        })?;
                    let link = if link.is_absolute() {
                        link
                    } else {
                        current.parent().unwrap_or(Path::new(".")).join(link)
                    };
                    if link.exists() {
                        fs::sync::link_dir(&link, &target)?;
                    }
                }
                Ok(metadata) if metadata.is_dir() => {
                    let entry = self.store.adopt(package, &current)?;
                    if let Err(error) = fs::sync::link_dir(entry.path(), &current) {
                        let _ = fs::sync::rename(entry.path(), &current);
                        return Err(error.into());
                    }
                    if let Err(error) = fs::sync::link_dir(entry.path(), &target) {
                        let _ = fs::sync::remove_file(&current);
                        let _ = fs::sync::rename(entry.path(), &current);
                        return Err(error.into());
                    }
                }
                Ok(_) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(fs::Error::Metadata {
                        path: current,
                        source,
                    }
                    .into());
                }
            }
        }

        slot.rebuild_views()?;
        Ok(StagingTree { slot })
    }

    fn rebuild_views(&self) -> Result<(), Error> {
        let lockfile = self.lockfile()?;
        for (id, package) in lockfile.rocks() {
            if self.view_layout_for(&package.spec).root.is_dir() {
                self.finalize(package, lockfile.entry_type(id))?;
            }
        }
        Ok(())
    }

    fn link_entry(&self, entry: &Entry) -> Result<(), Error> {
        let target = self.view_layout_for(entry.package()).root;
        if target
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            fs::sync::remove_dir_all(&target)?;
        }
        Ok(fs::sync::link_dir(entry.path(), &target)?)
    }

    fn view_layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        RockLayout::new(self.root.join(format!(
            "{}-{}@{}",
            package.id(),
            package.name(),
            package.version()
        )))
    }

    fn pending_entry(&self, id: &LockedPackageId) -> Option<Entry> {
        self.pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(id)
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
}

fn move_legacy_tree(from: &Path, to: &Path, version: &lua::Version) -> Result<(), Error> {
    if !from.is_dir() || from.is_symlink() {
        return Ok(());
    }

    if !to.exists() {
        fs::sync::rename(from, to)?;
        return Ok(());
    }

    let from = from.join(version.to_string());
    let to = to.join(version.to_string());
    if from.is_dir() && !to.exists() {
        fs::sync::rename(from, to)?;
    }
    Ok(())
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

fn available_tree_dir(base: &Path, seed: impl AsRef<[u8]>) -> PathBuf {
    let hash = hex::encode(Sha256::digest(seed.as_ref()));
    for suffix in 0.. {
        let name = if suffix == 0 {
            format!(".tree-{hash}")
        } else {
            format!(".tree-{hash}-{suffix}")
        };
        let path = base.join(name);
        if path.symlink_metadata().is_err() {
            return path;
        }
    }

    unreachable!()
}

fn slot_name_is_staging(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(".tree-"))
}

fn link_binaries(src: &Path, dest: &Path) -> Result<(), Error> {
    link_binaries_inner(src, dest, true)
}

fn link_binaries_inner(src: &Path, dest: &Path, top_level: bool) -> Result<(), Error> {
    if !src.is_dir() {
        return Ok(());
    }

    for entry in fs::sync::read_dir(src)? {
        let entry = entry?;
        if top_level && entry.file_name() == OsStr::new("unwrapped") {
            continue;
        }

        let path = entry.path();
        let target = dest.join(entry.file_name());
        if entry.file_type().map_err(Error::Io)?.is_dir() {
            link_binaries_inner(&path, &target, false)?;
        } else {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            fs::sync::link_file(path, &target)?;
        }
    }

    Ok(())
}

impl InstallTree for Tree {
    fn version(&self) -> &lua::Version {
        self.slot.version()
    }

    fn root(&self) -> PathBuf {
        self.slot.root()
    }

    fn bin(&self) -> PathBuf {
        self.slot.bin()
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.slot.unwrapped_bin()
    }

    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        self.slot.layout_for(package)
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        self.slot.lockfile()
    }

    fn lockfile_path(&self) -> PathBuf {
        self.slot.lockfile_path()
    }

    fn build_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.slot.build_tree(config)
    }

    fn test_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.slot.test_tree(config)
    }

    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        self.slot.list()
    }

    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
        self.slot.match_rocks(req)
    }

    fn prepare(&self, package: &LockedPackage) -> Result<(), Error> {
        self.slot.prepare(package)
    }

    fn finalize(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.slot.finalize(package, entry_type)
    }

    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.slot.cleanup(package, entry_type)
    }
}

impl Stageable for Tree {
    fn stage(&self) -> Result<StagingTree, Error> {
        self.staging()
    }

    fn commit_stage(&self, staging: &StagingTree) -> Result<(), Error> {
        self.commit(staging)
    }

    fn abort_stage(&self, staging: &StagingTree) {
        self.abort(staging)
    }
}

impl Installable for StagingTree {}

impl InstallTree for StagingTree {
    fn version(&self) -> &lua::Version {
        self.slot.version()
    }

    fn root(&self) -> PathBuf {
        self.slot.root()
    }

    fn bin(&self) -> PathBuf {
        self.slot.bin()
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.slot.unwrapped_bin()
    }

    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        self.slot.layout_for(package)
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        self.slot.lockfile()
    }

    fn lockfile_path(&self) -> PathBuf {
        self.slot.lockfile_path()
    }

    fn build_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.slot.build_tree(config)
    }

    fn test_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.slot.test_tree(config)
    }

    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        self.slot.list()
    }

    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
        self.slot.match_rocks(req)
    }

    fn prepare(&self, package: &LockedPackage) -> Result<(), Error> {
        self.slot.prepare(package)
    }

    fn finalize(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.slot.finalize(package, entry_type)
    }

    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.slot.cleanup(package, entry_type)
    }
}

impl TreeSlot {
    fn version(&self) -> &lua::Version {
        &self.version
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.bin().join("unwrapped")
    }

    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        if let Some(entry) = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&package.id())
        {
            entry.layout()
        } else {
            self.view_layout_for(package)
        }
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        Ok(Lockfile::new(self.lockfile_path())?)
    }

    fn lockfile_path(&self) -> PathBuf {
        self.root.join(LOCKFILE_NAME)
    }

    fn build_tree(&self, config: &Config) -> Result<Tree, Error> {
        match self.kind {
            TreeKind::Build => Ok(Tree::from_slot(self.clone())),
            _ => Tree::open(
                self.store.clone(),
                self.version.clone(),
                self.tree_root.clone(),
                TreeKind::Build,
                config,
            ),
        }
    }

    fn test_tree(&self, config: &Config) -> Result<Tree, Error> {
        match self.kind {
            TreeKind::Test => Ok(Tree::from_slot(self.clone())),
            _ => Tree::open(
                self.store.clone(),
                self.version.clone(),
                self.tree_root.clone(),
                TreeKind::Test,
                config,
            ),
        }
    }

    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        Ok(self.lockfile()?.list())
    }

    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
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

    fn prepare(&self, package: &LockedPackage) -> Result<(), Error> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if pending.contains_key(&package.id()) {
            return Ok(());
        }

        let entry = self.store.entry(package)?;
        entry.prepare()?;
        pending.insert(package.id(), entry);
        Ok(())
    }

    fn finalize(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        if let Some(entry) = self.pending_entry(&package.id()) {
            self.link_entry(&entry)?;
        }

        let layout = self.view_layout_for(&package.spec);
        link_binaries(&layout.bin, &self.bin())?;

        if let Some(custom_layout) = &self.entrypoint_layout {
            if entry_type.is_entrypoint() || custom_layout.layout_dependencies() {
                custom_layout.apply_layout(self, &package.spec)?;
            }
        }

        Ok(())
    }

    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        if let Some(layout) = &self.entrypoint_layout {
            if entry_type.is_entrypoint() || layout.layout_dependencies() {
                layout.remove_layout(self, &package.spec)?;
            }
        }

        if let Some(entry) = self.pending_entry(&package.id()) {
            let _ = fs::sync::remove_dir_all(entry.path());
        }

        let layout = self.view_layout_for(&package.spec);
        fs::sync::remove_path(&layout.root)?;

        let lockfile = self.lockfile()?;
        for relative_binary_path in lockfile
            .binaries(&package.id())
            .into_iter()
            .flat_map(|binaries| binaries.iter())
        {
            let binary_path = self.bin().join(relative_binary_path);
            if binary_path.symlink_metadata().is_ok() {
                fs::sync::remove_file(binary_path)?;
            }

            let unwrapped_binary_path = self.unwrapped_bin().join(relative_binary_path);
            if unwrapped_binary_path.symlink_metadata().is_ok() {
                fs::sync::remove_file(unwrapped_binary_path)?;
            }
        }

        Ok(())
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
    use assert_fs::prelude::PathCopy;
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
        tree::{InstallTree, RockLayout},
        variables,
    };

    #[test]
    fn rock_layout() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

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
                bin: tree_path.join(format!("5.1/{id}-neorg@8.0.0-1/bin")),
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
    fn staging_commit_replaces_pointer() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let pointer = tree.root();
        let previous = std::fs::read_link(&pointer).unwrap();

        let transaction = Transaction::new(&tree).unwrap();
        fs::sync::write(transaction.root().join("marker"), "marker").unwrap();
        transaction.commit().unwrap();

        let current = std::fs::read_link(&pointer).unwrap();
        let current = pointer.parent().unwrap().join(current);
        assert!(current.join("marker").is_file());
        assert!(!pointer.parent().unwrap().join(previous).exists());
    }

    #[test]
    fn staged_binaries_link_to_the_store() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
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

        let transaction = Transaction::new(&tree).unwrap();
        transaction.prepare(&package).unwrap();
        let layout = transaction.layout_for(&package.spec);
        fs::sync::write(layout.bin.join("tool"), "tool").unwrap();
        transaction
            .finalize(&package, crate::tree::EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let binary = tree.bin().join("tool");
        assert!(binary.is_file());
        let resolved = std::fs::canonicalize(binary).unwrap();
        assert!(resolved.starts_with(std::fs::canonicalize(temp.path()).unwrap().join("store")));
    }

    #[test]
    fn legacy_build_tree_migration_is_per_lua_version() {
        let temp = assert_fs::TempDir::new().unwrap();
        let root = temp.path();
        let base = root.join("build_dependencies");
        fs::sync::create_dir_all(base.join("5.1")).unwrap();

        let from = root.join("5.2/build_dependencies");
        fs::sync::create_dir_all(from.join("5.2")).unwrap();
        fs::sync::write(from.join("5.2/marker"), "marker").unwrap();

        super::move_legacy_tree(&from, &base, &lua::Version::Lua52).unwrap();

        assert!(base.join("5.1").is_dir());
        assert!(base.join("5.2/marker").is_file());
    }

    #[test]
    fn staging_adopts_and_links_existing_packages() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");
        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let lockfile = tree.lockfile().unwrap();
        let package = lockfile
            .rocks()
            .values()
            .find(|package| tree.layout_for(&package.spec).root.is_dir())
            .cloned()
            .unwrap();
        let current = tree.layout_for(&package.spec).root;
        assert!(!current.is_symlink());

        let transaction = Transaction::new(&tree).unwrap();
        assert!(current.is_symlink());
        assert!(current.is_dir());
        assert!(transaction.layout_for(&package.spec).root.is_symlink());
        assert!(transaction.layout_for(&package.spec).root.is_dir());
        let staging_root = transaction.root();

        drop(transaction);
        assert!(current.is_dir());
        assert!(!staging_root.exists());
    }

    #[test]
    fn tree_list() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
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
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");

        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

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
