use bon::Builder;

use crate::{
    build::BuildBehaviour,
    lockfile::{self, OptState, PinnedState},
    package, rockspec, tree,
};

/// Specifies how to install a package
#[derive(Debug, Builder, PartialEq, Eq, Hash, Clone)]
#[builder(start_fn = new, finish_fn(name = build, vis = "pub"))]
pub struct Spec {
    #[builder(start_fn)]
    pub(crate) package: package::Req,
    #[builder(start_fn)]
    pub(crate) entry_type: tree::EntryType,
    pub(crate) build_behaviour: BuildBehaviour,
    #[builder(default)]
    pub(crate) pin: PinnedState,
    #[builder(default)]
    pub(crate) opt: OptState,
    pub(crate) source: Option<rockspec::source::Spec>,
    /// Optional constraint, carried over from a previous install,
    /// e.g. defined in a lockfile.
    pub(crate) constraint: Option<lockfile::Constraint>,
}
