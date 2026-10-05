use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use tokio::task::JoinSet;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{
        LockedPackageId, LockedPackageLockType, LockedPackageSpec, OptState, PinnedState,
        RemotePackageSourceUrl,
    },
    lua_rockspec::{BuildBackendSpec, RemoteLuaRockspec},
    operations::PackageInstallSpec,
    package::{PackageName, PackageReq},
    remote_package_db::RemotePackageDB,
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
    tree::EntryType,
};

use super::{
    discover::{DiscoverError, FoundPackage},
    download_sources_and_hash::PackageSource,
    Artifacts,
};

/// The build dependencies of a rockspec that still need to be installed, with the
/// LuaRocks build backend (if any) first.
pub(crate) fn build_dependencies_to_install<R: Rockspec>(rockspec: &R) -> Vec<PackageName> {
    let mut names = rockspec
        .build_dependencies()
        .current_platform()
        .iter()
        .filter(|dep| {
            !matches!(
                dep.name().to_string().as_str(),
                "luarocks-build-rust-mlua"
                    | "luarocks-build-rust-binary"
                    | "luarocks-build-treesitter-parser"
            )
        })
        .map(|dep| dep.name().clone())
        .collect_vec();

    if let Some(backend) = luarocks_build_backend_name(rockspec) {
        names.insert(0, backend);
    }
    names
}

/// The name of the luarocks build backend rock required to build this rockspec (if any).
pub(crate) fn luarocks_build_backend_name<R: Rockspec>(rockspec: &R) -> Option<PackageName> {
    match &rockspec.build().current_platform().build_backend {
        Some(BuildBackendSpec::LuaRock(backend)) => {
            Some(PackageName::new(format!("luarocks-build-{backend}")))
        }
        _ => None,
    }
}

pub(crate) struct ResolvedPackage {
    pub(crate) spec: LockedPackageSpec,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) source: RemotePackageSource,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
    pub(crate) entry_type: EntryType,
    pub(crate) build_behaviour: crate::build::BuildBehaviour,
    pub(crate) artifact: Option<PackageSource>,
}

pub(crate) type ResolvedArtifacts = Artifacts<HashMap<LockedPackageId, ResolvedPackage>>;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum ResolveError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Discover(#[from] DiscoverError),
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependency(String),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct ResolvePackageDependencies<'a> {
    #[builder(start_fn)]
    pub(crate) package_db: RemotePackageDB,
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    #[builder(field)]
    pub(crate) packages: Artifacts<Vec<PackageInstallSpec>>,
}

impl<State> ResolvePackageDependenciesBuilder<'_, State>
where
    State: resolve_package_dependencies_builder::State,
{
    fn set(mut self, section: LockedPackageLockType, packages: Vec<PackageInstallSpec>) -> Self {
        *self.packages.get_mut(section) = Some(packages);
        self
    }

    pub(crate) fn packages(self, packages: Vec<PackageInstallSpec>) -> Self {
        self.set(LockedPackageLockType::Regular, packages)
    }

    pub(crate) fn build_packages(self, packages: Vec<PackageInstallSpec>) -> Self {
        self.set(LockedPackageLockType::Build, packages)
    }

    pub(crate) fn test_packages(self, packages: Vec<PackageInstallSpec>) -> Self {
        self.set(LockedPackageLockType::Test, packages)
    }
}

impl<State> ResolvePackageDependenciesBuilder<'_, State>
where
    State: resolve_package_dependencies_builder::State
        + resolve_package_dependencies_builder::IsComplete,
{
    pub(crate) async fn resolve(self) -> Result<ResolvedArtifacts, ResolveError> {
        let args = self._build();
        Resolver::new(args.package_db, args.config)
            .run(args.packages)
            .await
    }
}

/// Identifies a single discovery request. Two requests with the same key resolve to the
/// same package, so the diamond problem is solved by only discovering each key once.
#[derive(Clone, PartialEq, Eq, Hash)]
struct RequestKey {
    section: LockedPackageLockType,
    package: PackageReq,
    pin: PinnedState,
    opt: OptState,
}

impl RequestKey {
    fn from_install_spec(section: LockedPackageLockType, spec: &PackageInstallSpec) -> Self {
        Self {
            section,
            package: spec.package.clone(),
            pin: spec.pin,
            opt: spec.opt,
        }
    }
}

/// A discovered node in the dependency graph.
struct Node {
    install_spec: PackageInstallSpec,
    id: LockedPackageId,
    found: FoundPackage,
    children: Vec<(bool, RequestKey)>,
}

struct Resolver {
    package_db: RemotePackageDB,
    config: Arc<Config>,
    max_concurrent: usize,
    joinset: JoinSet<(
        RequestKey,
        PackageInstallSpec,
        Result<FoundPackage, ResolveError>,
    )>,
    pending: VecDeque<(RequestKey, PackageInstallSpec)>,
    seen: HashSet<RequestKey>,
    nodes: HashMap<RequestKey, Node>,
    resolved: ResolvedArtifacts,
}

impl Resolver {
    fn new(package_db: RemotePackageDB, config: &Config) -> Self {
        Self::with_max_concurrent(package_db, Arc::new(config.clone()), config.max_jobs())
    }

    fn with_max_concurrent(
        package_db: RemotePackageDB,
        config: Arc<Config>,
        max_concurrent: usize,
    ) -> Self {
        Self {
            package_db,
            config,
            max_concurrent,
            joinset: JoinSet::new(),
            pending: VecDeque::new(),
            seen: HashSet::new(),
            nodes: HashMap::new(),
            resolved: ResolvedArtifacts::default(),
        }
    }

    async fn run(
        mut self,
        packages: Artifacts<Vec<PackageInstallSpec>>,
    ) -> Result<ResolvedArtifacts, ResolveError> {
        for (section, specs) in packages {
            let Some(specs) = specs else {
                continue;
            };
            // Mark the section as requested even if it resolves to nothing.
            self.resolved
                .get_mut(section)
                .get_or_insert_with(HashMap::new);
            for spec in specs {
                let key = RequestKey::from_install_spec(section, &spec);
                self.enqueue(key, spec);
            }
        }

        self.spawn_ready();
        while let Some(joined) = self.joinset.join_next().await {
            let (key, spec, result) = joined.map_err(ResolveError::Join)?;
            self.on_discovered(key, spec, result?);
            self.spawn_ready();
        }

        detect_cycles(&self.nodes)?;
        Ok(self.assemble())
    }

    /// Registers a request for discovery, unless it has already been seen.
    fn enqueue(&mut self, key: RequestKey, spec: PackageInstallSpec) {
        if self.seen.insert(key.clone()) {
            self.pending.push_back((key, spec));
        }
    }

    /// Spawns queued discoveries up to the configured concurrency limit.
    fn spawn_ready(&mut self) {
        while self.joinset.len() < self.max_concurrent {
            let Some((key, spec)) = self.pending.pop_front() else {
                break;
            };
            let package_db = self.package_db.clone();
            let config = Arc::clone(&self.config);
            let task_spec = spec.clone();
            self.joinset.spawn(async move {
                let result = package_db
                    .resolve(&task_spec, &config)
                    .await
                    .map_err(ResolveError::Discover);
                (key, spec, result)
            });
        }
    }

    /// Records a discovered package and queues any children it introduces.
    fn on_discovered(&mut self, key: RequestKey, spec: PackageInstallSpec, found: FoundPackage) {
        let section = key.section;
        let is_binary = matches!(
            found.package.source,
            RemotePackageSource::LuarocksBinaryRock(_)
        );
        let constraint = spec
            .constraint
            .clone()
            .unwrap_or_else(|| spec.package.version_req().clone().into());
        let id = LockedPackageId::new(
            found.package.package.name(),
            found.package.package.version(),
            spec.pin,
            spec.opt,
            constraint,
        );

        let mut child_specs: Vec<(bool, RequestKey, PackageInstallSpec)> = Vec::new();
        if !is_binary {
            for child_spec in build_dependency_specs(&found.rockspec, &spec) {
                push_child(&mut child_specs, true, section, child_spec);
            }
        }
        for dependency in found.rockspec.dependencies().current_platform() {
            let child_spec = PackageInstallSpec::new(
                dependency.package_req().clone(),
                EntryType::DependencyOnly,
            )
            .build_behaviour(BuildBehaviour::Ignore)
            .pin(spec.pin)
            .opt(spec.opt)
            .maybe_source(dependency.source().clone())
            .build();
            push_child(&mut child_specs, false, section, child_spec);
        }

        let children = child_specs
            .iter()
            .map(|(is_build, child_key, _)| (*is_build, child_key.clone()))
            .collect();
        self.resolved
            .get_mut(section)
            .get_or_insert_with(HashMap::new);
        self.nodes.insert(
            key,
            Node {
                install_spec: spec,
                id,
                found,
                children,
            },
        );

        for (_, child_key, child_spec) in child_specs {
            self.enqueue(child_key, child_spec);
        }
    }

    /// Computes the final specs and assembles the resolved packages now that discovery is
    /// complete and every child id is known.
    fn assemble(self) -> ResolvedArtifacts {
        let Resolver {
            nodes,
            mut resolved,
            ..
        } = self;

        let ids: HashMap<RequestKey, LockedPackageId> = nodes
            .iter()
            .map(|(key, node)| (key.clone(), node.id.clone()))
            .collect();

        for (key, node) in nodes {
            let Node {
                install_spec,
                id,
                found,
                children,
            } = node;
            let dependencies = children
                .iter()
                .filter(|(is_build, _)| !is_build)
                .map(|(_, child)| ids[child].clone())
                .collect_vec();
            let build_dependencies = children
                .iter()
                .filter(|(is_build, _)| *is_build)
                .map(|(_, child)| ids[child].clone())
                .collect_vec();
            let constraint = install_spec
                .constraint
                .clone()
                .unwrap_or_else(|| install_spec.package.version_req().clone().into());

            let spec = LockedPackageSpec::new(
                found.package.package.name(),
                found.package.package.version(),
                constraint,
                dependencies,
                build_dependencies,
                &install_spec.pin,
                &install_spec.opt,
            );
            let package = ResolvedPackage {
                spec,
                rockspec: found.rockspec,
                source: found.package.source,
                source_url: found.package.source_url,
                entry_type: install_spec.entry_type,
                build_behaviour: install_spec.build_behaviour,
                artifact: found.artifact,
            };

            resolved
                .get_mut(key.section)
                .get_or_insert_with(HashMap::new)
                .entry(id)
                .or_insert(package);
        }

        resolved
    }
}

/// Adds a child request unless an equivalent request is already present.
fn push_child(
    children: &mut Vec<(bool, RequestKey, PackageInstallSpec)>,
    is_build: bool,
    section: LockedPackageLockType,
    spec: PackageInstallSpec,
) {
    let child_section = if is_build {
        LockedPackageLockType::Build
    } else {
        section
    };
    let child_key = RequestKey::from_install_spec(child_section, &spec);
    if !children
        .iter()
        .any(|(_, existing, _)| existing == &child_key)
    {
        children.push((is_build, child_key, spec));
    }
}

fn detect_cycles(nodes: &HashMap<RequestKey, Node>) -> Result<(), ResolveError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Grey,
        Black,
    }

    let mut colors: HashMap<&RequestKey, Color> = HashMap::with_capacity(nodes.len());

    for start in nodes.keys() {
        if colors.get(start).copied().unwrap_or(Color::White) != Color::White {
            continue;
        }
        colors.insert(start, Color::Grey);
        let mut stack: Vec<(&RequestKey, usize)> = vec![(start, 0)];
        let mut path: Vec<&RequestKey> = vec![start];

        while let Some(&(node, index)) = stack.last() {
            let children = &nodes[node].children;
            if index < children.len() {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                let child = &children[index].1;
                match colors.get(child).copied().unwrap_or(Color::White) {
                    Color::White => {
                        colors.insert(child, Color::Grey);
                        path.push(child);
                        stack.push((child, 0));
                    }
                    Color::Grey => {
                        let Some(position) = path.iter().position(|key| *key == child) else {
                            continue;
                        };
                        let chain = path[position..]
                            .iter()
                            .map(|key| key.package.name().to_string())
                            .chain(std::iter::once(child.package.name().to_string()))
                            .collect_vec()
                            .join(" -> ");
                        return Err(ResolveError::CyclicDependency(chain));
                    }
                    Color::Black => {}
                }
            } else {
                colors.insert(node, Color::Black);
                stack.pop();
                path.pop();
            }
        }
    }

    Ok(())
}

fn build_dependency_specs(
    rockspec: &RemoteLuaRockspec,
    parent: &PackageInstallSpec,
) -> Vec<PackageInstallSpec> {
    build_dependencies_to_install(rockspec)
        .into_iter()
        .filter_map(|name| {
            rockspec
                .build_dependencies()
                .current_platform()
                .iter()
                .find(|dep| dep.name() == &name)
                .map(|dep| {
                    PackageInstallSpec::new(dep.package_req().clone(), EntryType::Entrypoint)
                        .build_behaviour(BuildBehaviour::Ignore)
                        .pin(parent.pin)
                        .opt(parent.opt)
                        .maybe_source(dep.source().clone())
                        .build()
                })
                .or_else(|| {
                    Some(
                        PackageInstallSpec::new(PackageReq::from(name), EntryType::Entrypoint)
                            .build_behaviour(BuildBehaviour::Ignore)
                            .pin(parent.pin)
                            .opt(parent.opt)
                            .build(),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::ConfigBuilder;
    use crate::package::{PackageSpec, RemotePackage};
    use crate::remote_package_db::RemoteSource;

    fn rockspec(package: &str, dependencies: &[&str]) -> String {
        let dependencies = if dependencies.is_empty() {
            String::new()
        } else {
            let list = dependencies
                .iter()
                .map(|dependency| format!("'{dependency}'"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("\ndependencies = {{ {list} }}")
        };
        format!(
            "rockspec_format = '1.0'\npackage = '{package}'\nversion = '1.0.0-1'\nsource = {{ url = 'https://example.com/{package}.zip' }}{dependencies}"
        )
    }

    fn local_packages(rockspecs: &[String]) -> RemotePackageDB {
        RemotePackageDB(vec![RemoteSource::Local(
            rockspecs
                .iter()
                .map(|content| {
                    let rockspec = match RemoteLuaRockspec::new(content) {
                        Ok(rockspec) => rockspec,
                        Err(err) => panic!("invalid test rockspec: {err}"),
                    };
                    FoundPackage {
                        package: RemotePackage::new(
                            PackageSpec::new(
                                rockspec.package().clone(),
                                rockspec.version().clone(),
                            ),
                            RemotePackageSource::RockspecContent(content.clone()),
                            None,
                        ),
                        rockspec,
                        artifact: None,
                    }
                })
                .collect(),
        )])
    }

    fn install_spec(name: &str) -> PackageInstallSpec {
        let package = match name.parse::<PackageReq>() {
            Ok(package) => package,
            Err(err) => panic!("invalid package req: {err}"),
        };
        PackageInstallSpec::new(package, EntryType::Entrypoint)
            .build_behaviour(BuildBehaviour::Ignore)
            .build()
    }

    fn roots(spec: PackageInstallSpec) -> Artifacts<Vec<PackageInstallSpec>> {
        Artifacts {
            regular: Some(vec![spec]),
            build: None,
            test: None,
        }
    }

    fn test_config() -> Arc<Config> {
        let builder = match ConfigBuilder::new() {
            Ok(builder) => builder,
            Err(err) => panic!("invalid test config: {err}"),
        };
        let config = match builder.build() {
            Ok(config) => config,
            Err(err) => panic!("invalid test config: {err}"),
        };
        Arc::new(config)
    }

    #[tokio::test]
    async fn diamond_problem() {
        let packages = local_packages(&[
            rockspec("a", &["b", "c"]),
            rockspec("b", &["d"]),
            rockspec("c", &["d"]),
            rockspec("d", &[]),
        ]);

        let resolved = match Resolver::with_max_concurrent(packages, test_config(), 8)
            .run(roots(install_spec("a")))
            .await
        {
            Ok(resolved) => resolved,
            Err(err) => panic!("resolution failed: {err}"),
        };

        assert_eq!(resolved.regular.as_ref().map(|rocks| rocks.len()), Some(4));
    }

    #[tokio::test]
    async fn cyclic_dependencies_are_rejected() {
        let packages = local_packages(&[rockspec("a", &["b"]), rockspec("b", &["a"])]);

        let result = Resolver::with_max_concurrent(packages, test_config(), 8)
            .run(roots(install_spec("a")))
            .await;

        assert!(matches!(result, Err(ResolveError::CyclicDependency(_))));
    }
}
