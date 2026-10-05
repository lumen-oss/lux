use bon::Builder;
use tracing::Instrument;

use crate::{
    build::{deploy, BuildBehaviour, BuildError, RemotePackageSourceSpec},
    config::Config,
    hash::HasIntegrity,
    lockfile::{LockConstraint, LockedPackage, LockedPackageSpec, OptState, PinnedState},
    lua_installation::LuaInstallation,
    remote_package_source::RemotePackageSource,
    rockspec::{LuaVersionCompatibility, Rockspec},
    tree::{EntryType, InstallTree},
};

use super::download_sources_and_hash::fetch_and_hash_source;

/// A rocks package builder, providing fine-grained control
/// over how a package should be built.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Build<'a, R: Rockspec + HasIntegrity, T: InstallTree> {
    rockspec: &'a R,
    tree: &'a T,
    entry_type: EntryType,
    config: &'a Config,
    lua: &'a LuaInstallation,

    #[builder(default)]
    pin: PinnedState,
    #[builder(default)]
    opt: OptState,
    #[builder(default)]
    constraint: LockConstraint,
    behaviour: BuildBehaviour,

    #[builder(setters(vis = "pub(crate)"))]
    source_spec: Option<RemotePackageSourceSpec>,

    // TODO(vhyrro): Remove this and enforce that this is provided at a type level.
    #[builder(setters(vis = "pub(crate)"))]
    source: Option<RemotePackageSource>,
}

impl<R: Rockspec + HasIntegrity, T: InstallTree + Sync, State> BuildBuilder<'_, R, T, State>
where
    State: build_builder::State + build_builder::IsComplete,
{
    pub async fn build(self) -> Result<LockedPackage, BuildError> {
        let build = self._build();
        let span = tracing::info_span!(
            "Building",
            package = build.rockspec.package().to_string(),
            version = build.rockspec.version().to_string(),
        );
        do_build(build).instrument(span).await
    }
}

#[tracing::instrument(level = "trace", skip_all)]
async fn do_build<R, T>(build: Build<'_, R, T>) -> Result<LockedPackage, BuildError>
where
    R: Rockspec + HasIntegrity,
    T: InstallTree + Sync,
{
    let rockspec = build.rockspec;
    let lua = build.lua;

    rockspec.validate_lua_version(&lua.version)?;

    let spec = LockedPackageSpec::new(
        rockspec.package(),
        rockspec.version(),
        build.constraint,
        Vec::new(),
        Vec::new(),
        &build.pin,
        &build.opt,
    );
    // Pin the futures otherwise they fill up the stack quite a bit
    let (package, temp_dir) = Box::pin(fetch_and_hash_source(
        rockspec,
        spec,
        build.source_spec,
        build.source,
        build.config,
    ))
    .await?;

    Box::pin(deploy(
        rockspec,
        build.tree,
        package,
        lua,
        temp_dir.path(),
        build.entry_type,
        build.config,
        build.behaviour,
    ))
    .await
}
