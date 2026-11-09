use crate::lockfile::locked_package;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, ErrorKind, Write},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::{
    fs,
    lockfile::{
        load_lockfile, Error, LockedPackage, LockedPackageLockType, LockfilePermissions,
        PackageLock, PackageSyncSpec, ReadOnly, ReadWrite, SyncStrategy, LOCKFILE_VERSION_STR,
    },
    rockspec::lua_dependency,
};

/// A lockfile for a Lua project
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lockfile<P: LockfilePermissions> {
    #[serde(skip)]
    filepath: PathBuf,
    #[serde(skip)]
    _marker: PhantomData<P>,
    version: String,
    #[serde(default, skip_serializing_if = "PackageLock::is_empty")]
    dependencies: PackageLock,
    #[serde(default, skip_serializing_if = "PackageLock::is_empty")]
    test_dependencies: PackageLock,
    #[serde(default, skip_serializing_if = "PackageLock::is_empty")]
    build_dependencies: PackageLock,
}

impl<P: LockfilePermissions> Lockfile<P> {
    pub(crate) fn rocks(
        &self,
        deps: &LockedPackageLockType,
    ) -> &BTreeMap<locked_package::ID, LockedPackage> {
        match deps {
            LockedPackageLockType::Regular => self.dependencies.rocks(),
            LockedPackageLockType::Test => self.test_dependencies.rocks(),
            LockedPackageLockType::Build => self.build_dependencies.rocks(),
        }
    }

    pub(crate) fn get(
        &self,
        id: &locked_package::ID,
        deps: &LockedPackageLockType,
    ) -> Option<&LockedPackage> {
        match deps {
            LockedPackageLockType::Regular => self.dependencies.get(id),
            LockedPackageLockType::Test => self.test_dependencies.get(id),
            LockedPackageLockType::Build => self.build_dependencies.get(id),
        }
    }

    pub(crate) fn is_entrypoint(
        &self,
        package: &locked_package::ID,
        deps: &LockedPackageLockType,
    ) -> bool {
        match deps {
            LockedPackageLockType::Regular => self.dependencies.is_entrypoint(package),
            LockedPackageLockType::Test => self.test_dependencies.is_entrypoint(package),
            LockedPackageLockType::Build => self.build_dependencies.is_entrypoint(package),
        }
    }

    pub(crate) fn package_sync_spec(
        &self,
        packages: &[lua_dependency::Spec],
        deps: &LockedPackageLockType,
        strategy: &SyncStrategy<'_>,
    ) -> PackageSyncSpec {
        match deps {
            LockedPackageLockType::Regular => {
                self.dependencies.package_sync_spec(packages, strategy)
            }
            LockedPackageLockType::Test => {
                self.test_dependencies.package_sync_spec(packages, strategy)
            }
            LockedPackageLockType::Build => self
                .build_dependencies
                .package_sync_spec(packages, strategy),
        }
    }

    pub(crate) fn local_pkg_lock(&self, deps: &LockedPackageLockType) -> &PackageLock {
        match deps {
            LockedPackageLockType::Regular => &self.dependencies,
            LockedPackageLockType::Test => &self.test_dependencies,
            LockedPackageLockType::Build => &self.build_dependencies,
        }
    }

    pub(crate) fn local_pkg_locks(&self) -> Vec<PackageLock> {
        vec![
            self.dependencies.clone(),
            self.test_dependencies.clone(),
            self.build_dependencies.clone(),
        ]
    }

    fn flush(&self) -> io::Result<()> {
        let content = serde_json::to_string_pretty(&self)?;

        fs::sync::write_atomic(&self.filepath, content).map_err(io::Error::other)?;

        Ok(())
    }
}

impl Lockfile<ReadOnly> {
    /// Create a new `project::Lockfile`, writing an empty file if none exists.
    #[tracing::instrument(level = "trace")]
    pub fn new(filepath: PathBuf) -> Result<Lockfile<ReadOnly>, Error> {
        // Ensure that the lockfile exists
        match File::options().create_new(true).write(true).open(&filepath) {
            Ok(mut file) => {
                let empty_lockfile: Lockfile<ReadOnly> = Lockfile {
                    filepath: filepath.clone(),
                    _marker: PhantomData,
                    version: LOCKFILE_VERSION_STR.into(),
                    dependencies: PackageLock::default(),
                    test_dependencies: PackageLock::default(),
                    build_dependencies: PackageLock::default(),
                };
                let json_str = serde_json::to_string(&empty_lockfile).map_err(Error::WriteJson)?;
                write!(file, "{json_str}").map_err(|source| fs::Error::Write {
                    path: filepath.to_path_buf(),
                    source,
                })?;
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(Error::Fs(fs::Error::FileOpen {
                    path: filepath.to_path_buf(),
                    source,
                }));
            }
        }

        Self::load(filepath)
    }

    /// Load a `project::Lockfile`, failing if none exists.
    #[tracing::instrument(level = "trace")]
    pub fn load(filepath: PathBuf) -> Result<Lockfile<ReadOnly>, Error> {
        let content = fs::sync::read_to_string(&filepath)?;
        let mut lockfile: Lockfile<ReadOnly> = load_lockfile(
            &content,
            &filepath,
            "this lockfile was created by an incompatible version of Lux.
remove the `lux.lock` and run `lx sync` to regenerate it.",
        )?;

        lockfile.filepath = filepath;

        Ok(lockfile)
    }

    /// Creates a temporary, writeable project lockfile which can never flush.
    fn into_temporary(self) -> Lockfile<ReadWrite> {
        Lockfile::<ReadWrite> {
            _marker: PhantomData,
            filepath: self.filepath,
            version: self.version,
            dependencies: self.dependencies,
            test_dependencies: self.test_dependencies,
            build_dependencies: self.build_dependencies,
        }
    }

    /// Creates a project lockfile guard, flushing the lockfile automatically
    /// once the guard goes out of scope.
    pub fn write_guard(self) -> Guard {
        Guard(self.into_temporary())
    }
}

impl Lockfile<ReadWrite> {
    pub(crate) fn remove(&mut self, target: &LockedPackage, deps: &LockedPackageLockType) {
        match deps {
            LockedPackageLockType::Regular => self.dependencies.remove(target),
            LockedPackageLockType::Test => self.test_dependencies.remove(target),
            LockedPackageLockType::Build => self.build_dependencies.remove(target),
        }
    }

    pub(crate) fn sync(&mut self, lock: &PackageLock, deps: &LockedPackageLockType) {
        match deps {
            LockedPackageLockType::Regular => {
                self.dependencies = lock.clone();
            }
            LockedPackageLockType::Test => {
                self.test_dependencies = lock.clone();
            }
            LockedPackageLockType::Build => {
                self.build_dependencies = lock.clone();
            }
        }
    }

    pub(crate) fn merge(&mut self, lock: &PackageLock, deps: &LockedPackageLockType) {
        match deps {
            LockedPackageLockType::Regular => self.dependencies.merge(lock),
            LockedPackageLockType::Test => self.test_dependencies.merge(lock),
            LockedPackageLockType::Build => self.build_dependencies.merge(lock),
        }
    }
}

pub struct Guard(Lockfile<ReadWrite>);

impl Serialize for Guard {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Guard {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Guard(Lockfile::<ReadWrite>::deserialize(deserializer)?))
    }
}

impl Deref for Guard {
    type Target = Lockfile<ReadWrite>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn migrate_v1_workspace_lockfile() {
        let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/lockfile-v1/workspace.lock");

        let temp = assert_fs::TempDir::new().unwrap();
        let lockfile_path = temp.path().join("lux.lock");
        std::fs::copy(&sample, &lockfile_path).unwrap();

        let lockfile = Lockfile::load(lockfile_path.clone()).unwrap();

        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&lockfile_path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], "1.0.0");
        assert!(on_disk["dependencies"]["entrypoints"].is_array());

        assert_eq!(lockfile.version, "2.0.0");
        assert_eq!(lockfile.dependencies.entrypoints.len(), 3);
    }
}
