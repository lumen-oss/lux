use crate::package;
use crate::rockspec;
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::PathBuf,
    string::FromUtf8Error,
};

use bon::Builder;
use bytes::Bytes;
use miette::Diagnostic;
use thiserror::Error;
use url::{ParseError, Url};

use crate::{
    config::Config,
    fs, git,
    lockfile::RemotePackageSourceUrl,
    luarocks,
    package::db::{search, PackageDB},
    remote,
    reqwest::RequestBuilderExt,
    rockspec::Rockspec as _,
};

/// Builder for a rock downloader.
pub struct Download<'a> {
    package_req: &'a package::Req,
    package_db: Option<&'a PackageDB>,
    config: &'a Config,
}

impl<'a> Download<'a> {
    /// Construct a new `.src.rock` downloader.
    pub fn new(package_req: &'a package::Req, config: &'a Config) -> Self {
        Self {
            package_req,
            package_db: None,
            config,
        }
    }

    /// Sets the package database to use for searching for packages.
    /// Instantiated from the config if not set.
    pub fn package_db(self, package_db: &'a PackageDB) -> Self {
        Self {
            package_db: Some(package_db),
            ..self
        }
    }

    /// Download the package's Rockspec.
    pub async fn download_rockspec(self) -> Result<Rockspec, Error> {
        match self.package_db {
            Some(db) => download_rockspec(self.package_req, db, self.config).await,
            None => {
                let db = PackageDB::from_config(self.config).await?;
                download_rockspec(self.package_req, &db, self.config).await
            }
        }
    }

    /// Download a `.src.rock` to a file.
    /// `destination_dir` defaults to the current working directory if not set.
    pub async fn download_src_rock_to_file(
        self,
        destination_dir: Option<PathBuf>,
    ) -> Result<PackedRock, Error> {
        match self.package_db {
            Some(db) => {
                download_src_rock_to_file(self.package_req, destination_dir, db, self.config).await
            }
            None => {
                let db = PackageDB::from_config(self.config).await?;
                download_src_rock_to_file(self.package_req, destination_dir, &db, self.config).await
            }
        }
    }

    /// Search for a `.src.rock` and download it to memory.
    pub async fn search_and_download_src_rock(self) -> Result<PackedRockBytes, Error> {
        match self.package_db {
            Some(db) => search_and_download_src_rock(self.package_req, db, self.config).await,
            None => {
                let db = PackageDB::from_config(self.config).await?;
                search_and_download_src_rock(self.package_req, &db, self.config).await
            }
        }
    }

    pub(crate) async fn download_remote_rock(self) -> Result<RemoteRock, Error> {
        match self.package_db {
            Some(db) => download_remote_rock(self.package_req, db, self.config).await,
            None => {
                let db = PackageDB::from_config(self.config).await?;
                download_remote_rock(self.package_req, &db, self.config).await
            }
        }
    }
}

pub struct PackedRockBytes {
    pub name: package::Name,
    pub version: package::Version,
    pub bytes: Bytes,
    pub file_name: String,
    pub url: Url,
}

pub struct PackedRock {
    pub name: package::Name,
    pub version: package::Version,
    pub path: PathBuf,
}

/// Remote Lua RockSpec that has been downloaded from a remote server, along with its source metadata
#[derive(Clone, Debug)]
pub struct Rockspec {
    pub rockspec: rockspec::lua::Remote,
    pub(crate) source: remote::package::Source,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
}

#[derive(Clone, Debug)]
pub(crate) enum RemoteRock {
    RockspecOnly {
        rockspec_download: Rockspec,
    },
    BinaryRock {
        rockspec_download: Rockspec,
        packed_rock: Bytes,
    },
    SrcRock {
        rockspec_download: Rockspec,
        src_rock: Bytes,
        source_url: RemotePackageSourceUrl,
    },
}

impl RemoteRock {
    pub fn rockspec(&self) -> &rockspec::lua::Remote {
        &self.rockspec_download().rockspec
    }
    pub fn rockspec_download(&self) -> &Rockspec {
        match self {
            Self::RockspecOnly { rockspec_download }
            | Self::BinaryRock {
                rockspec_download, ..
            }
            | Self::SrcRock {
                rockspec_download, ..
            } => rockspec_download,
        }
    }

    fn into_rockspec_download(self) -> Rockspec {
        match self {
            Self::RockspecOnly { rockspec_download }
            | Self::BinaryRock {
                rockspec_download, ..
            }
            | Self::SrcRock {
                rockspec_download, ..
            } => rockspec_download,
        }
    }

    // Instead of downloading a rockspec, generate one from a `Req` and a `source::Spec`.
    pub(crate) fn from_package_req_and_source_spec(
        package_req: package::Req,
        source_spec: rockspec::source::Spec,
    ) -> Result<Self, Error> {
        let package_spec: package::Spec = package_req.try_into()?;
        let source_url = Some(match &source_spec {
            rockspec::source::Spec::Git(git::Source { url, git_ref }) => {
                let checkout_ref = match git_ref {
                    Some(git::Ref::Tag(tag)) => tag.clone(),
                    Some(git::Ref::Branch(branch)) => branch.clone(),
                    None => return Err(Error::MissingCheckoutRef(url.to_string())),
                };
                RemotePackageSourceUrl::Git {
                    url: url.to_string(),
                    checkout_ref,
                    submodules: false, // unknown until the source is fetched
                }
            }
            rockspec::source::Spec::File(path) => {
                RemotePackageSourceUrl::File { path: path.clone() }
            }
            rockspec::source::Spec::Url(url) => RemotePackageSourceUrl::Url { url: url.clone() },
            rockspec::source::Spec::Wally(_) => {
                return Err(Error::WallySource(package_spec.name().to_string()))
            }
        });
        let rockspec = rockspec::lua::Remote::from_source_spec(
            package_spec,
            source_spec,
            Vec::new(),
            HashMap::new(),
        );
        let rockspec_content = rockspec.to_lua_remote_rockspec_string()?;
        let rockspec_download = Rockspec {
            rockspec,
            source_url,
            source: remote::package::Source::RockspecContent(rockspec_content),
        };
        Ok(Self::RockspecOnly { rockspec_download })
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum RockspecError {
    #[error("failed to download rockspec")]
    #[diagnostic(transparent)]
    Request(#[from] crate::reqwest::Error),
    #[error("failed to convert rockspec response")]
    #[diagnostic(help(
        r#"the server returned a response that is not valid UTF-8.
check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    ResponseConversion(#[from] FromUtf8Error),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    PackageDB(#[from] package::db::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    DownloadSrcRock(#[from] SrcRockError),
}

/// Find and download a rockspec for a given package requirement
async fn download_rockspec(
    package_req: &package::Req,
    package_db: &PackageDB,
    config: &Config,
) -> Result<Rockspec, Error> {
    Ok(download_remote_rock(package_req, package_db, config)
        .await?
        .into_rockspec_download())
}

pub(crate) async fn exact_rockspec(
    package: &package::Spec,
    package_db: &PackageDB,
    config: &Config,
) -> Result<Rockspec, Error> {
    let package_req = package.clone().into();
    let remote = package_db.find(&package_req, None).await?;
    let remote_package = remote::Package {
        spec: package.clone(),
        ..remote
    };
    Ok(download_found_rock(remote_package, config)
        .await?
        .into_rockspec_download())
}

#[tracing::instrument(
    name = "Downloading rock",
    level = "info",
    skip_all,
    fields(package = package_req.to_string(),),
)]
async fn download_remote_rock(
    package_req: &package::Req,
    package_db: &PackageDB,
    config: &Config,
) -> Result<RemoteRock, Error> {
    let remote_package = package_db.find(package_req, None).await?;
    download_found_rock(remote_package, config).await
}

async fn download_found_rock(
    remote_package: remote::Package,
    config: &Config,
) -> Result<RemoteRock, Error> {
    match &remote_package.source {
        remote::package::Source::LuarocksRockspec(url) => {
            let package = &remote_package.spec;
            let rockspec_name = format!("{}-{}.rockspec", package.name(), package.version());
            let url = url.join(&rockspec_name).map_err(|source| Error::Parse {
                source,
                url: format!("{}/{}", url, rockspec_name),
            })?;
            let bytes = crate::reqwest::https_client(config)?
                .get(url.clone())
                .apply_access_token(config, &url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            let content = String::from_utf8(bytes.into())?;
            let rockspec = Rockspec {
                rockspec: rockspec::lua::Remote::new(&content)
                    .map_err(|err| Error::Rockspec(err.into()))?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRock::RockspecOnly {
                rockspec_download: rockspec,
            })
        }
        remote::package::Source::RockspecContent(content) => {
            let rockspec = Rockspec {
                rockspec: rockspec::lua::Remote::new(content)
                    .map_err(|err| Error::Rockspec(err.into()))?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRock::RockspecOnly {
                rockspec_download: rockspec,
            })
        }
        remote::package::Source::LuarocksBinaryRock(url) => {
            // prioritise lockfile source_url
            let url = if let Some(RemotePackageSourceUrl::Url { url }) = &remote_package.source_url
            {
                url
            } else {
                url
            };
            let rock = binary_rock(&remote_package.spec, url, config).await?;
            let rockspec = Rockspec {
                rockspec: unpack_rockspec(&rock).await?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRock::BinaryRock {
                rockspec_download: rockspec,
                packed_rock: rock.bytes,
            })
        }
        remote::package::Source::LuarocksSrcRock(url) => {
            // prioritise lockfile source_url
            let url = if let Some(RemotePackageSourceUrl::Url { url }) = &remote_package.source_url
            {
                url.clone()
            } else {
                url.clone()
            };
            let rock = src_rock(&remote_package.spec, &url, config).await?;
            let rockspec = Rockspec {
                rockspec: unpack_rockspec(&rock).await?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRock::SrcRock {
                rockspec_download: rockspec,
                src_rock: rock.bytes,
                source_url: RemotePackageSourceUrl::Url { url },
            })
        }
        remote::package::Source::Wally(_) => {
            Err(Error::WallySource(remote_package.spec.name().to_string()))
        }
        remote::package::Source::Local => Err(Error::LocalSource),
        #[cfg(test)]
        remote::package::Source::Test => unimplemented!(),
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("failed to parse rock URL '{url}'")]
    Parse {
        source: url::ParseError,
        url: String,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Search(#[from] search::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Download(#[from] SrcRockError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    DownloadRockspec(#[from] RockspecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("UTF-8 conversion failed")]
    #[diagnostic(help(
        r#"the server returned a response that is not valid UTF-8.
check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Utf8(#[from] FromUtf8Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Rockspec(#[from] rockspec::Error),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    PackageDB(#[from] package::db::Error),
    #[error("failed to read packed rock {0}:\n{1}")]
    #[diagnostic(help(
        r#"the downloaded rock may be corrupted.
check your network connection and rerun the command to re-download it."#
    ))]
    ZipRead(String, zip::result::ZipError),
    #[error("failed to extract packed rock {0}:\n{1}")]
    #[diagnostic(help(
        r#"the downloaded rock may be corrupted.
check your network connection and rerun the command to re-download it."#
    ))]
    ZipExtract(String, zip::result::ZipError),
    #[error("{0} not found in the packed rock.")]
    #[diagnostic(help(
        r#"the packed rock does not contain the expected rockspec.
check your network connection and rerun the command to re-download it."#
    ))]
    RockspecNotFoundInPackedRock(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageSpecFromPackageReq(#[from] crate::package::spec::Error),
    #[error("git source {0} without a revision or tag.")]
    #[diagnostic(help(
        r#"lux requires a pinned revision or tag to ensure reproducible builds.
without one, the same version could resolve to different code at different times.
try a different version, or report it to the package maintainer."#
    ))]
    MissingCheckoutRef(String),
    #[error("cannot download from a local rock source.")]
    #[diagnostic(
        help(
            r#"lux cannot download rocks from a local path.
for local dependencies, use `path` in your lux.toml."#
        ),
        url("https://lux.lumen-labs.org/reference/lux-toml#local-dependencies")
    )]
    LocalSource,
    #[error("cannot download from a local rock or embedded rockspec source.")]
    #[diagnostic(
        help(
            r#"lux cannot download rocks from a local path.
for local dependencies, use `path` in your lux.toml."#
        ),
        url("https://lux.lumen-labs.org/reference/lux-toml#local-dependencies")
    )]
    NonURLSource,
    #[error("failed to resolve wally package '{0}'")]
    #[diagnostic(
        code(lux_lib::wally::unresolved_source),
        help("this is a bug, please report it.")
    )]
    WallySource(String),
    #[error("client error")]
    #[diagnostic(transparent)]
    Request(#[from] crate::reqwest::Error),
}

async fn search_and_download_src_rock(
    package_req: &package::Req,
    package_db: &PackageDB,
    config: &Config,
) -> Result<PackedRockBytes, Error> {
    let filter = Some(remote::package::kind::FilterSpec {
        rockspec: false,
        binary: false,
        src: true,
    });
    let remote_package = package_db.find(package_req, filter).await?;
    let source_url = remote_package.source.url().ok_or(Error::NonURLSource)?;
    Ok(src_rock(&remote_package.spec, &source_url, config).await?)
}

#[derive(Error, Debug, Diagnostic)]
pub enum SrcRockError {
    #[error("failed to download source rock")]
    #[diagnostic(transparent)]
    Request(#[from] crate::reqwest::Error),
    #[error("failed to parse source rock URL")]
    Parse(#[from] ParseError),
}

#[tracing::instrument(name = "Downloading src.rock", skip_all)]
pub(crate) async fn src_rock(
    package: &package::Spec,
    server_url: &Url,
    config: &Config,
) -> Result<PackedRockBytes, SrcRockError> {
    ArchiveDownload::new()
        .package(package)
        .server_url(server_url)
        .config(config)
        .ext("src.rock")
        .download()
        .await
}

#[tracing::instrument(name = "Downloading binary rock", skip_all)]
pub(crate) async fn binary_rock(
    package: &package::Spec,
    server_url: &Url,
    config: &Config,
) -> Result<PackedRockBytes, SrcRockError> {
    let ext = format!("{}.rock", luarocks::current_platform_luarocks_identifier());
    ArchiveDownload::new()
        .package(package)
        .server_url(server_url)
        .config(config)
        .ext(&ext)
        .fallback_ext("all.rock")
        .download()
        .await
}

#[tracing::instrument(name = "Downloading package", skip_all)]
async fn download_src_rock_to_file(
    package_req: &package::Req,
    destination_dir: Option<PathBuf>,
    package_db: &PackageDB,
    config: &Config,
) -> Result<PackedRock, Error> {
    let rock = search_and_download_src_rock(package_req, package_db, config).await?;
    let full_rock_name = mk_packed_rock_name(&rock.name, &rock.version, "src.rock");
    fs::tokio::write(
        destination_dir
            .map(|dest| dest.join(&full_rock_name))
            .unwrap_or_else(|| full_rock_name.clone().into()),
        &rock.bytes,
    )
    .await?;

    Ok(PackedRock {
        name: rock.name.to_owned(),
        version: rock.version.to_owned(),
        path: full_rock_name.into(),
    })
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
struct ArchiveDownload<'a> {
    package: &'a package::Spec,

    server_url: &'a Url,

    ext: &'a str,

    fallback_ext: Option<&'a str>,

    config: &'a Config,
}

impl<State> ArchiveDownloadBuilder<'_, State>
where
    State: archive_download_builder::State + archive_download_builder::IsComplete,
{
    async fn download(self) -> Result<PackedRockBytes, SrcRockError> {
        let args = self._build();
        download_impl(args).await
    }
}

#[tracing::instrument(
    name = "Downloading",
    level = "info",
    skip_all,
    fields(
        package = args.package.name().to_string(),
        version = args.package.version().to_string(),
        server = args.server_url.to_string(),
    ),
)]
async fn download_impl(args: ArchiveDownload<'_>) -> Result<PackedRockBytes, SrcRockError> {
    let package = args.package;
    let ext = args.ext;
    let server_url = args.server_url;
    let full_rock_name = mk_packed_rock_name(package.name(), package.version(), ext);
    tracing::debug!(message = format!("📥 Downloading {full_rock_name}").as_str());
    let url = server_url.join(&full_rock_name)?;
    let response = crate::reqwest::https_client(args.config)?
        .get(url.clone())
        .apply_access_token(args.config, &url)
        .send()
        .await?;
    let bytes = if response.status().is_success() {
        response.bytes().await
    } else {
        match args.fallback_ext {
            Some(ext) => {
                let full_rock_name = mk_packed_rock_name(package.name(), package.version(), ext);
                let url = server_url.join(&full_rock_name)?;
                crate::reqwest::https_client(args.config)?
                    .get(url.clone())
                    .apply_access_token(args.config, &url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await
            }
            None => response.error_for_status()?.bytes().await,
        }
    }?;
    Ok(PackedRockBytes {
        name: package.name().clone(),
        version: package.version().clone(),
        bytes,
        file_name: full_rock_name,
        url,
    })
}
fn mk_packed_rock_name(name: &package::Name, version: &package::Version, ext: &str) -> String {
    format!("{name}-{version}.{ext}")
}

pub(crate) async fn unpack_rockspec(
    rock: &PackedRockBytes,
) -> Result<rockspec::lua::Remote, Error> {
    let cursor = Cursor::new(&rock.bytes);
    let rockspec_file_name = format!("{}-{}.rockspec", rock.name, rock.version);
    let mut zip =
        zip::ZipArchive::new(cursor).map_err(|err| Error::ZipRead(rock.file_name.clone(), err))?;
    let rockspec_index = (0..zip.len())
        .find(|&i| {
            unsafe { zip.by_index(i).unwrap_unchecked() }
                .name()
                .is_ok_and(|name| name.eq(&rockspec_file_name))
        })
        .ok_or(Error::RockspecNotFoundInPackedRock(
            rockspec_file_name.to_string(),
        ))?;
    let mut rockspec_file = zip
        .by_index(rockspec_index)
        .map_err(|err| Error::ZipExtract(rock.file_name.clone(), err))?;
    let mut content = String::new();
    rockspec_file
        .read_to_string(&mut content)
        .map_err(|source| fs::Error::ReadToString {
            path: PathBuf::from(&rockspec_file_name),
            source,
        })?;
    let rockspec =
        rockspec::lua::Remote::new(&content).map_err(|err| Error::Rockspec(err.into()))?;
    Ok(rockspec)
}

impl From<reqwest::Error> for RockspecError {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

impl From<reqwest::Error> for SrcRockError {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}
