#[cfg(feature = "wally")]
pub(crate) mod wally;

use std::path::PathBuf;

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config,
    lua_rockspec::{RemoteLuaRockspec, RockSourceSpec},
    operations,
    operations::{Download, FetchVendored, PackageInstallSpec, RemoteRockDownload},
    package::{PackageReq, PackageSpec, RemotePackage},
    package_db::{PackageDB, SearchError},
    project::Project,
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
};

use super::materialise::PackageSource;

#[derive(Clone, Debug)]
pub(crate) struct FoundPackage {
    pub(crate) remote: RemotePackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) artifact: Option<PackageSource>,
}

impl FoundPackage {
    /// Builds a discovered package from an already-parsed rockspec, so it can
    /// be resolved as a root without contacting a remote package database.
    pub(crate) fn from_rockspec(rockspec: RemoteLuaRockspec) -> Self {
        let source = rockspec
            .to_lua_remote_rockspec_string()
            .map(RemotePackageSource::RockspecContent)
            .unwrap_or(RemotePackageSource::Local);
        let remote = RemotePackage::new(
            PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
            source,
            None,
        );
        Self {
            remote,
            rockspec,
            artifact: None,
        }
    }

    /// Builds a discovered package from a project root rockspec, using the
    /// project's own directory as the source.
    pub(crate) fn from_project_root(rockspec: RemoteLuaRockspec, root_dir: PathBuf) -> Self {
        let remote = RemotePackage::new(
            PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
            RemotePackageSource::Local,
            None,
        );
        Self {
            remote,
            rockspec,
            artifact: Some(PackageSource::SourceDir(root_dir)),
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum DiscoverError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Search(#[from] SearchError),
    #[error("failed to acquire metadata for '{0}'")]
    #[diagnostic(forward(1))]
    Download(PackageReq, Box<operations::SearchAndDownloadError>),
    #[error("failed to fetch vendored package for '{0}'")]
    #[diagnostic(forward(1))]
    FetchVendored(PackageReq, Box<operations::FetchVendoredError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] crate::project::ProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalRockspec(#[from] crate::project::IntoLocalRockspecError),
    #[cfg(feature = "wally")]
    #[error(transparent)]
    #[diagnostic(transparent)]
    Wally(#[from] wally::WallyDiscoverError),
    #[cfg(not(feature = "wally"))]
    #[error("wally support is disabled in this build")]
    #[diagnostic(help("this build of Lux was compiled without the `wally` feature"))]
    WallyDisabled,
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Discover<'a> {
    #[builder(start_fn)]
    pub(crate) package_db: PackageDB,
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
}

impl<State> DiscoverBuilder<'_, State>
where
    State: discover_builder::State + discover_builder::IsComplete,
{
    pub(crate) async fn discover(
        self,
        spec: &PackageInstallSpec,
    ) -> Result<FoundPackage, DiscoverError> {
        let args = self._build();
        if let Some(found) = discover_project(spec)? {
            return Ok(found);
        }
        if let Some(source) = &spec.source {
            return discover_source_spec(spec, source, args.config).await;
        }
        discover_package_db(spec, &args.package_db, args.config).await
    }
}

fn discover_project(spec: &PackageInstallSpec) -> Result<Option<FoundPackage>, DiscoverError> {
    let Some(RockSourceSpec::File(path)) = &spec.source else {
        return Ok(None);
    };
    Ok(match Project::from_exact(path)? {
        Some(project) if project.toml().package() == spec.package.name() => {
            Some(FoundPackage::from_project_root(
                project.local_remote_rockspec()?,
                project.root().to_path_buf(),
            ))
        }
        _ => None,
    })
}

async fn discover_source_spec(
    spec: &PackageInstallSpec,
    source: &RockSourceSpec,
    _config: &Config,
) -> Result<FoundPackage, DiscoverError> {
    match source {
        RockSourceSpec::Git(_) | RockSourceSpec::File(_) | RockSourceSpec::Url(_) => {
            discover_fetchable_source(spec, source.clone())
        }
        #[cfg(feature = "wally")]
        RockSourceSpec::Wally(wally_req) => wally::discover(&spec.package, wally_req, _config)
            .await
            .map_err(DiscoverError::Wally),
        #[cfg(not(feature = "wally"))]
        RockSourceSpec::Wally(_) => Err(DiscoverError::WallyDisabled),
    }
}

fn discover_fetchable_source(
    spec: &PackageInstallSpec,
    source: RockSourceSpec,
) -> Result<FoundPackage, DiscoverError> {
    let download =
        RemoteRockDownload::from_package_req_and_source_spec(spec.package.clone(), source)
            .map_err(|err| DiscoverError::Download(spec.package.clone(), Box::new(err)))?;
    let rockspec = download.rockspec().clone();
    let remote = RemotePackage::new(
        spec.package.clone().try_into().map_err(|err| {
            DiscoverError::Download(
                spec.package.clone(),
                Box::new(operations::SearchAndDownloadError::from(err)),
            )
        })?,
        download.rockspec_download().source.clone(),
        download.rockspec_download().source_url.clone(),
    );
    Ok(FoundPackage {
        remote,
        rockspec,
        artifact: None,
    })
}

async fn discover_package_db(
    spec: &PackageInstallSpec,
    package_db: &PackageDB,
    config: &Config,
) -> Result<FoundPackage, DiscoverError> {
    if let Some(found) = package_db.resolve(spec, config).await? {
        Ok(found)
    } else {
        download_remote(spec, package_db, config).await
    }
}

async fn download_remote(
    spec: &PackageInstallSpec,
    package_db: &PackageDB,
    config: &Config,
) -> Result<FoundPackage, DiscoverError> {
    if package_db.has_remote_sources() {
        let mut remote = package_db.find(&spec.package, None).await?;
        #[cfg(feature = "wally")]
        if let RemotePackageSource::Wally(index_url) = &remote.source {
            return wally::discover_locked(&remote, index_url, config)
                .await
                .map_err(DiscoverError::Wally);
        }
        #[cfg(not(feature = "wally"))]
        if let RemotePackageSource::Wally(_) = &remote.source {
            return Err(DiscoverError::WallyDisabled);
        }
        let download = if let Some(vendor_dir) = config.vendor_dir() {
            FetchVendored::new()
                .vendor_dir(vendor_dir)
                .package(&spec.package)
                .package_db(package_db)
                .fetch_vendored_rock()
                .await
                .map_err(|err| DiscoverError::FetchVendored(spec.package.clone(), Box::new(err)))?
        } else {
            Download::new(&spec.package, config)
                .package_db(package_db)
                .download_remote_rock()
                .await
                .map_err(|err| DiscoverError::Download(spec.package.clone(), Box::new(err)))?
        };
        let rockspec = download.rockspec().clone();
        let artifact = match &download {
            RemoteRockDownload::SrcRock {
                src_rock,
                source_url,
                ..
            } => {
                remote.source_url = Some(source_url.clone());
                Some(PackageSource::Archive(src_rock.clone()))
            }
            RemoteRockDownload::BinaryRock { packed_rock, .. } => {
                Some(PackageSource::Archive(packed_rock.clone()))
            }
            RemoteRockDownload::RockspecOnly { .. } => None,
        };
        Ok(FoundPackage {
            remote,
            rockspec,
            artifact,
        })
    } else {
        Err(DiscoverError::Search(SearchError::RockNotFound(
            spec.package.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::{build::BuildBehaviour, config::ConfigBuilder, tree::EntryType};

    #[test]
    fn from_rockspec_builds_local_found_package() {
        let content = "rockspec_format = '1.0'\npackage = 'foo'\nversion = '1.0.0-1'\nsource = { url = 'https://example.com/foo.zip' }";
        let Ok(rockspec) = RemoteLuaRockspec::new(content) else {
            unreachable!("invalid test rockspec");
        };

        let found = FoundPackage::from_rockspec(rockspec);

        assert_eq!(found.remote.spec.name().to_string(), "foo");
        assert_eq!(found.remote.spec.version().to_string(), "1.0.0-1");
        assert!(matches!(
            found.remote.source,
            RemotePackageSource::RockspecContent(_)
        ));
        assert!(found.artifact.is_none());
    }

    // https://github.com/lumen-oss/lux/issues/2018
    #[tokio::test]
    async fn resolves_path_dependency_from_project_manifest() {
        let bar_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/multi-project-local-deps/projects/bar");

        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let spec = PackageInstallSpec::new(
            PackageReq::new("bar".into(), Some("1.0.0".into())).unwrap(),
            EntryType::DependencyOnly,
        )
        .build_behaviour(BuildBehaviour::Ignore)
        .maybe_source(Some(RockSourceSpec::File(bar_dir)))
        .build();

        let found = Discover::new(PackageDB::from_local(Vec::new()), &config)
            .discover(&spec)
            .await
            .unwrap();

        assert!(found
            .rockspec
            .dependencies()
            .current_platform()
            .iter()
            .any(|dependency| dependency.name().to_string() == "foo"));
    }
}
