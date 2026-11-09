use std::fmt::Display;

use serde::{de, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ssri::Integrity;

use super::constraint;
use super::{Constraint, IntegrityError, OptState, PinnedState, RemotePackageSourceUrl};
use crate::{package, remote};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Spec {
    name: package::Name,
    version: package::Version,
    pub(super) pinned: PinnedState,
    opt: OptState,
    pub(super) dependencies: Vec<ID>,
    pub(super) build_dependencies: Vec<ID>,
    constraint: Option<String>,
}

/// ID of a local package, a hash that is comprised of:
/// - name
/// - version
/// - pinned state
/// - opt state
/// - lock constraint
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Clone)]
pub struct ID(String);

impl ID {
    pub fn new(
        name: &package::Name,
        version: &package::Version,
        pinned: PinnedState,
        opt: OptState,
        constraint: Constraint,
    ) -> Self {
        let mut hasher = Sha256::new();

        hasher.update(format!(
            "{}{}{}{}{}",
            name,
            version,
            pinned.as_bool(),
            opt.as_bool(),
            match constraint {
                Constraint::Unconstrained => String::default(),
                Constraint::Constrained(version_req) => version_req.to_string(),
            },
        ));

        Self(hex::encode(hasher.finalize()))
    }

    /// Constructs a package ID from a hashed string.
    ///
    /// # Safety
    ///
    /// Ensure that the hash you are providing to this function
    /// is not malformed and resolves to a valid package ID for the target
    /// tree you are working with.
    pub unsafe fn from_unchecked(str: String) -> Self {
        Self(str)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Display for ID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Spec {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: &package::Name,
        version: &package::Version,
        constraint: Constraint,
        dependencies: Vec<ID>,
        build_dependencies: Vec<ID>,
        pinned: &PinnedState,
        opt: &OptState,
    ) -> Self {
        Self {
            name: name.clone(),
            version: version.clone(),
            pinned: *pinned,
            opt: *opt,
            dependencies,
            build_dependencies,
            constraint: match constraint {
                Constraint::Unconstrained => None,
                Constraint::Constrained(version_req) => Some(version_req.to_string()),
            },
        }
    }

    pub fn id(&self) -> ID {
        ID::new(
            self.name(),
            self.version(),
            self.pinned,
            self.opt,
            match &self.constraint {
                None => Constraint::Unconstrained,
                Some(_) => self.constraint(),
            },
        )
    }

    pub fn constraint(&self) -> Constraint {
        unsafe { Constraint::try_from(&self.constraint).unwrap_unchecked() }
    }

    pub fn name(&self) -> &package::Name {
        &self.name
    }

    pub fn version(&self) -> &package::Version {
        &self.version
    }

    pub fn pinned(&self) -> PinnedState {
        self.pinned
    }

    pub fn opt(&self) -> OptState {
        self.opt
    }

    pub fn dependencies(&self) -> Vec<&ID> {
        self.dependencies.iter().collect()
    }

    pub fn build_dependencies(&self) -> Vec<&ID> {
        self.build_dependencies.iter().collect()
    }

    pub fn to_package(&self) -> package::Spec {
        package::Spec::new(self.name.clone(), self.version.clone())
    }

    pub fn into_package_req(self) -> package::Req {
        match self.constraint() {
            Constraint::Unconstrained => self.name.into(),
            Constraint::Constrained(version_req) => package::Req {
                name: self.name,
                version_req,
            },
        }
    }

    pub(crate) fn as_package_req(&self) -> package::Req {
        match self.constraint() {
            Constraint::Unconstrained => self.name.clone().into(),
            Constraint::Constrained(version_req) => package::Req {
                name: self.name.clone(),
                version_req: version_req.clone(),
            },
        }
    }
}

/// A package whose source has been downloaded, hashed and which is installed in a tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LockedPackage {
    pub(crate) spec: Spec,
    pub(crate) source: remote::package::Source,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
    pub(crate) hashes: Hashes,
}

impl LockedPackage {
    pub(crate) fn repin(mut self, pinned: PinnedState) -> Self {
        self.spec.pinned = pinned;
        self
    }

    #[cfg(test)]
    pub(crate) fn from(
        package: &package::Spec,
        constraint: Constraint,
        source: remote::package::Source,
        source_url: Option<RemotePackageSourceUrl>,
        hashes: Hashes,
    ) -> Self {
        Self::new(
            Spec::new(
                package.name(),
                package.version(),
                constraint,
                Vec::default(),
                Vec::default(),
                &PinnedState::Unpinned,
                &OptState::Required,
            ),
            source,
            source_url,
            hashes,
        )
    }

    pub(crate) fn new(
        spec: Spec,
        source: remote::package::Source,
        source_url: Option<RemotePackageSourceUrl>,
        hashes: Hashes,
    ) -> Self {
        Self {
            spec,
            source,
            source_url,
            hashes,
        }
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn into_package_spec(self) -> package::Spec {
        self.spec.to_package()
    }

    pub fn as_package_spec(&self) -> package::Spec {
        self.spec.to_package()
    }

    pub fn id(&self) -> ID {
        self.spec.id()
    }

    pub fn name(&self) -> &package::Name {
        self.spec.name()
    }

    pub fn version(&self) -> &package::Version {
        self.spec.version()
    }

    pub fn pinned(&self) -> PinnedState {
        self.spec.pinned()
    }

    pub fn opt(&self) -> OptState {
        self.spec.opt()
    }

    pub fn dependencies(&self) -> Vec<&ID> {
        self.spec.dependencies()
    }

    pub fn build_dependencies(&self) -> Vec<&ID> {
        self.spec.build_dependencies()
    }

    pub fn constraint(&self) -> Constraint {
        self.spec.constraint()
    }

    pub(crate) fn source(&self) -> &remote::package::Source {
        &self.source
    }

    pub(crate) fn source_url(&self) -> Option<&RemotePackageSourceUrl> {
        self.source_url.as_ref()
    }

    pub fn hashes(&self) -> &Hashes {
        &self.hashes
    }

    pub fn to_package(&self) -> package::Spec {
        self.spec.to_package()
    }

    pub fn into_package_req(self) -> package::Req {
        self.spec.into_package_req()
    }
}

impl TryFrom<LockedPackageIntermediate> for LockedPackage {
    type Error = constraint::Error;

    fn try_from(value: LockedPackageIntermediate) -> Result<Self, Self::Error> {
        let constraint = Constraint::try_from(&value.constraint)?;
        Ok(Self {
            spec: Spec::new(
                &value.name,
                &value.version,
                constraint,
                value.dependencies,
                value.build_dependencies,
                &value.pinned,
                &value.opt,
            ),
            source: value.source,
            source_url: value.source_url,
            hashes: value.hashes,
        })
    }
}

impl From<&LockedPackage> for LockedPackageIntermediate {
    fn from(value: &LockedPackage) -> Self {
        Self {
            name: value.spec.name.clone(),
            version: value.spec.version.clone(),
            pinned: value.spec.pinned,
            opt: value.spec.opt,
            dependencies: value.spec.dependencies.clone(),
            build_dependencies: value.spec.build_dependencies.clone(),
            constraint: value.spec.constraint.clone(),
            source: value.source.clone(),
            source_url: value.source_url.clone(),
            hashes: value.hashes.clone(),
        }
    }
}

impl<'de> Deserialize<'de> for LockedPackage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        LockedPackage::try_from(LockedPackageIntermediate::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

impl Serialize for LockedPackage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        LockedPackageIntermediate::from(self).serialize(serializer)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Hash)]
pub struct Hashes {
    pub rockspec: Integrity,
    pub source: Integrity,
}

impl Ord for Hashes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = (self.rockspec.to_hex().1, self.source.to_hex().1);
        let b = (other.rockspec.to_hex().1, other.source.to_hex().1);
        a.cmp(&b)
    }
}

impl PartialOrd for Hashes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(super) fn integrity_err_not_found(package: &LockedPackage) -> IntegrityError {
    IntegrityError::PackageNotFound(
        package.name().clone(),
        Box::new(package.version().clone()),
        package.spec.pinned,
        package
            .spec
            .constraint
            .clone()
            .unwrap_or("UNCONSTRAINED".into()),
    )
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LockedPackageIntermediate {
    name: package::Name,
    version: package::Version,
    #[serde(default, skip_serializing_if = "PinnedState::is_default")]
    pinned: PinnedState,
    #[serde(default, skip_serializing_if = "OptState::is_default")]
    opt: OptState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) dependencies: Vec<ID>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    build_dependencies: Vec<ID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    constraint: Option<String>,
    source: remote::package::Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_url: Option<RemotePackageSourceUrl>,
    hashes: Hashes,
}
