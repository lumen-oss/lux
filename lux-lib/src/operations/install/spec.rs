use bon::Builder;

use crate::{
    build::BuildBehaviour,
    lockfile::{LockConstraint, OptState, PinnedState},
    lua_rockspec::RockSourceSpec,
    package::PackageReq,
    tree,
};

/// Specifies how to install a package
#[derive(Debug, Builder, PartialEq, Eq, Hash, Clone)]
#[builder(start_fn = new, finish_fn(name = build, vis = "pub"))]
pub struct PackageInstallSpec {
    #[builder(start_fn)]
    pub(crate) package: PackageReq,
    #[builder(start_fn)]
    pub(crate) entry_type: tree::EntryType,
    #[builder(default)]
    pub(crate) build_behaviour: BuildBehaviour,
    #[builder(default)]
    pub(crate) pin: PinnedState,
    #[builder(default)]
    pub(crate) opt: OptState,
    pub(crate) source: Option<RockSourceSpec>,
    /// Optional constraint, carried over from a previous install,
    /// e.g. defined in a lockfile.
    pub(crate) constraint: Option<LockConstraint>,
}
