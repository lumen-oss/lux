use std::{
    io::{self, Cursor, Read},
    path::PathBuf,
    string::FromUtf8Error,
};

use bon::Builder;
use bytes::Bytes;
use thiserror::Error;
use url::{ParseError, Url};

use crate::{
    config::Config,
    git::GitSource,
    lockfile::RemotePackageSourceUrl,
    lua_rockspec::{LuaRockspecError, RemoteLuaRockspec, RockSourceSpec},
    luarocks,
    package::{
        PackageName, PackageReq, PackageSpec, PackageSpecFromPackageReqError, PackageVersion,
        RemotePackageTypeFilterSpec,
    },
    progress::{Progress, ProgressBar},
    remote_package_db::{RemotePackageDB, RemotePackageDBError, SearchError},
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
};

/// Builder for a rock downloader.
pub struct Download<'a> {
    package_req: &'a PackageReq,
    package_db: Option<&'a RemotePackageDB>,
    config: &'a Config,
    progress: &'a Progress<ProgressBar>,
}

impl<'a> Download<'a> {
    /// Construct a new `.src.rock` downloader.
    pub fn new(
        package_req: &'a PackageReq,
        config: &'a Config,
        progress: &'a Progress<ProgressBar>,
    ) -> Self {
        Self {
            package_req,
            package_db: None,
            config,
            progress,
        }
    }

    /// Sets the package database to use for searching for packages.
    /// Instantiated from the config if not set.
    pub fn package_db(self, package_db: &'a RemotePackageDB) -> Self {
        Self {
            package_db: Some(package_db),
            ..self
        }
    }

    /// Download the package's Rockspec.
    pub async fn download_rockspec(self) -> Result<DownloadedRockspec, SearchAndDownloadError> {
        match self.package_db {
            Some(db) => download_rockspec(self.package_req, db, self.progress).await,
            None => {
                let db = RemotePackageDB::from_config(self.config, self.progress).await?;
                download_rockspec(self.package_req, &db, self.progress).await
            }
        }
    }

    /// Download a `.src.rock` to a file.
    /// `destination_dir` defaults to the current working directory if not set.
    pub async fn download_src_rock_to_file(
        self,
        destination_dir: Option<PathBuf>,
    ) -> Result<DownloadedPackedRock, SearchAndDownloadError> {
        match self.package_db {
            Some(db) => {
                download_src_rock_to_file(self.package_req, destination_dir, db, self.progress)
                    .await
            }
            None => {
                let db = RemotePackageDB::from_config(self.config, self.progress).await?;
                download_src_rock_to_file(self.package_req, destination_dir, &db, self.progress)
                    .await
            }
        }
    }

    /// Search for a `.src.rock` and download it to memory.
    pub async fn search_and_download_src_rock(
        self,
    ) -> Result<DownloadedPackedRockBytes, SearchAndDownloadError> {
        match self.package_db {
            Some(db) => search_and_download_src_rock(self.package_req, db, self.progress).await,
            None => {
                let db = RemotePackageDB::from_config(self.config, self.progress).await?;
                search_and_download_src_rock(self.package_req, &db, self.progress).await
            }
        }
    }

    pub(crate) async fn download_remote_rock(
        self,
    ) -> Result<RemoteRockDownload, SearchAndDownloadError> {
        match self.package_db {
            Some(db) => download_remote_rock(self.package_req, db, self.progress).await,
            None => {
                let db = RemotePackageDB::from_config(self.config, self.progress).await?;
                download_remote_rock(self.package_req, &db, self.progress).await
            }
        }
    }
}

pub struct DownloadedPackedRockBytes {
    pub name: PackageName,
    pub version: PackageVersion,
    pub bytes: Bytes,
    pub file_name: String,
    pub url: Url,
}

pub struct DownloadedPackedRock {
    pub name: PackageName,
    pub version: PackageVersion,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct DownloadedRockspec {
    pub rockspec: RemoteLuaRockspec,
    pub(crate) source: RemotePackageSource,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
}

#[derive(Clone, Debug)]
pub(crate) enum RemoteRockDownload {
    RockspecOnly {
        rockspec_download: DownloadedRockspec,
    },
    BinaryRock {
        rockspec_download: DownloadedRockspec,
        packed_rock: Bytes,
    },
    SrcRock {
        rockspec_download: DownloadedRockspec,
        src_rock: Bytes,
        source_url: RemotePackageSourceUrl,
    },
}

impl RemoteRockDownload {
    pub fn rockspec(&self) -> &RemoteLuaRockspec {
        &self.rockspec_download().rockspec
    }
    pub fn rockspec_download(&self) -> &DownloadedRockspec {
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
    // Instead of downloading a rockspec, generate one from a `PackageReq` and a `RockSourceSpec`.
    pub(crate) fn from_package_req_and_source_spec(
        package_req: PackageReq,
        source_spec: RockSourceSpec,
    ) -> Result<Self, SearchAndDownloadError> {
        let package_spec = package_req.try_into()?;
        let source_url = Some(match &source_spec {
            RockSourceSpec::Git(GitSource { url, checkout_ref }) => RemotePackageSourceUrl::Git {
                url: url.to_string(),
                checkout_ref: checkout_ref
                    .clone()
                    .ok_or(SearchAndDownloadError::MissingCheckoutRef(url.to_string()))?,
            },
            RockSourceSpec::File(path) => RemotePackageSourceUrl::File { path: path.clone() },
            RockSourceSpec::Url(url) => RemotePackageSourceUrl::Url { url: url.clone() },
        });
        let rockspec = RemoteLuaRockspec::from_package_and_source_spec(package_spec, source_spec);
        let rockspec_content = rockspec
            .to_lua_remote_rockspec_string()
            .expect("the infallible happened");
        let rockspec_download = DownloadedRockspec {
            rockspec,
            source_url,
            source: RemotePackageSource::RockspecContent(rockspec_content),
        };
        Ok(Self::RockspecOnly { rockspec_download })
    }
}

#[derive(Error, Debug)]
pub enum DownloadRockspecError {
    #[error("failed to download rockspec: {0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to convert rockspec response: {0}")]
    ResponseConversion(#[from] FromUtf8Error),
    #[error("error initialising remote package DB: {0}")]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error(transparent)]
    DownloadSrcRock(#[from] DownloadSrcRockError),
}

/// Find and download a rockspec for a given package requirement
async fn download_rockspec(
    package_req: &PackageReq,
    package_db: &RemotePackageDB,
    progress: &Progress<ProgressBar>,
) -> Result<DownloadedRockspec, SearchAndDownloadError> {
    let rockspec = match download_remote_rock(package_req, package_db, progress).await? {
        RemoteRockDownload::RockspecOnly {
            rockspec_download: rockspec,
        } => rockspec,
        RemoteRockDownload::BinaryRock {
            rockspec_download: rockspec,
            ..
        } => rockspec,
        RemoteRockDownload::SrcRock {
            rockspec_download: rockspec,
            ..
        } => rockspec,
    };
    Ok(rockspec)
}

async fn download_remote_rock(
    package_req: &PackageReq,
    package_db: &RemotePackageDB,
    progress: &Progress<ProgressBar>,
) -> Result<RemoteRockDownload, SearchAndDownloadError> {
    let remote_package = package_db.find(package_req, None, progress).await?;
    progress.map(|p| p.set_message(format!("📥 Downloading rockspec for {package_req}")));
    match &remote_package.source {
        RemotePackageSource::LuarocksRockspec(url) => {
            let package = &remote_package.package;
            let rockspec_name = format!("{}-{}.rockspec", package.name(), package.version());
            let bytes = reqwest::Client::new()
                .get(format!("{}/{}", &url, rockspec_name))
                .send()
                .await
                .map_err(DownloadRockspecError::Request)?
                .error_for_status()
                .map_err(DownloadRockspecError::Request)?
                .bytes()
                .await
                .map_err(DownloadRockspecError::Request)?;
            let content = String::from_utf8(bytes.into())?;
            let rockspec = DownloadedRockspec {
                rockspec: RemoteLuaRockspec::new(&content)?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRockDownload::RockspecOnly {
                rockspec_download: rockspec,
            })
        }
        RemotePackageSource::RockspecContent(content) => {
            let rockspec = DownloadedRockspec {
                rockspec: RemoteLuaRockspec::new(content)?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRockDownload::RockspecOnly {
                rockspec_download: rockspec,
            })
        }
        RemotePackageSource::LuarocksBinaryRock(url) => {
            // prioritise lockfile source_url
            let url = if let Some(RemotePackageSourceUrl::Url { url }) = &remote_package.source_url
            {
                url
            } else {
                url
            };
            let rock = download_binary_rock(&remote_package.package, url, progress).await?;
            let rockspec = DownloadedRockspec {
                rockspec: unpack_rockspec(&rock).await?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRockDownload::BinaryRock {
                rockspec_download: rockspec,
                packed_rock: rock.bytes,
            })
        }
        RemotePackageSource::LuarocksSrcRock(url) => {
            // prioritise lockfile source_url
            let url = if let Some(RemotePackageSourceUrl::Url { url }) = &remote_package.source_url
            {
                url.clone()
            } else {
                url.clone()
            };
            let rock = download_src_rock(&remote_package.package, &url, progress).await?;
            let rockspec = DownloadedRockspec {
                rockspec: unpack_rockspec(&rock).await?,
                source: remote_package.source,
                source_url: remote_package.source_url,
            };
            Ok(RemoteRockDownload::SrcRock {
                rockspec_download: rockspec,
                src_rock: rock.bytes,
                source_url: RemotePackageSourceUrl::Url { url },
            })
        }
        RemotePackageSource::Local => Err(SearchAndDownloadError::LocalSource),
        #[cfg(test)]
        RemotePackageSource::Test => unimplemented!(),
    }
}

#[derive(Error, Debug)]
pub enum SearchAndDownloadError {
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error(transparent)]
    Download(#[from] DownloadSrcRockError),
    #[error(transparent)]
    DownloadRockspec(#[from] DownloadRockspecError),
    #[error("io operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("UTF-8 conversion failed: {0}")]
    Utf8(#[from] FromUtf8Error),
    #[error(transparent)]
    Rockspec(#[from] LuaRockspecError),
    #[error("error initialising remote package DB: {0}")]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to read packed rock {0}:\n{1}")]
    ZipRead(String, zip::result::ZipError),
    #[error("failed to extract packed rock {0}:\n{1}")]
    ZipExtract(String, zip::result::ZipError),
    #[error("{0} not found in the packed rock.")]
    RockspecNotFoundInPackedRock(String),
    #[error(transparent)]
    PackageSpecFromPackageReq(#[from] PackageSpecFromPackageReqError),
    #[error("git source {0} without a revision or tag.")]
    MissingCheckoutRef(String),
    #[error("cannot download from a local rock source.")]
    LocalSource,
    #[error("cannot download from a local rock or embedded rockspec source.")]
    NonURLSource,
}

async fn search_and_download_src_rock(
    package_req: &PackageReq,
    package_db: &RemotePackageDB,
    progress: &Progress<ProgressBar>,
) -> Result<DownloadedPackedRockBytes, SearchAndDownloadError> {
    let filter = Some(RemotePackageTypeFilterSpec {
        rockspec: false,
        binary: false,
        src: true,
    });
    let remote_package = package_db.find(package_req, filter, progress).await?;
    let source_url = remote_package
        .source
        .url()
        .ok_or(SearchAndDownloadError::NonURLSource)?;
    Ok(download_src_rock(&remote_package.package, &source_url, progress).await?)
}

#[derive(Error, Debug)]
pub enum DownloadSrcRockError {
    #[error("failed to download source rock: {0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to parse source rock URL: {0}")]
    Parse(#[from] ParseError),
}

pub(crate) async fn download_src_rock(
    package: &PackageSpec,
    server_url: &Url,
    progress: &Progress<ProgressBar>,
) -> Result<DownloadedPackedRockBytes, DownloadSrcRockError> {
    ArchiveDownload::new(package, server_url, "src.rock", progress)
        .download()
        .await
}

pub(crate) async fn download_binary_rock(
    package: &PackageSpec,
    server_url: &Url,
    progress: &Progress<ProgressBar>,
) -> Result<DownloadedPackedRockBytes, DownloadSrcRockError> {
    let ext = format!("{}.rock", luarocks::current_platform_luarocks_identifier());
    ArchiveDownload::new(package, server_url, &ext, progress)
        .fallback_ext("all.rock")
        .download()
        .await
}

async fn download_src_rock_to_file(
    package_req: &PackageReq,
    destination_dir: Option<PathBuf>,
    package_db: &RemotePackageDB,
    progress: &Progress<ProgressBar>,
) -> Result<DownloadedPackedRock, SearchAndDownloadError> {
    progress.map(|p| p.set_message(format!("📥 Downloading {package_req}")));

    let rock = search_and_download_src_rock(package_req, package_db, progress).await?;
    let full_rock_name = mk_packed_rock_name(&rock.name, &rock.version, "src.rock");
    tokio::fs::write(
        destination_dir
            .map(|dest| dest.join(&full_rock_name))
            .unwrap_or_else(|| full_rock_name.clone().into()),
        &rock.bytes,
    )
    .await?;

    Ok(DownloadedPackedRock {
        name: rock.name.to_owned(),
        version: rock.version.to_owned(),
        path: full_rock_name.into(),
    })
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
struct ArchiveDownload<'a> {
    #[builder(start_fn)]
    package: &'a PackageSpec,

    #[builder(start_fn)]
    server_url: &'a Url,

    #[builder(start_fn)]
    ext: &'a str,

    #[builder(start_fn)]
    progress: &'a Progress<ProgressBar>,

    fallback_ext: Option<&'a str>,
}

impl<State> ArchiveDownloadBuilder<'_, State>
where
    State: archive_download_builder::State,
{
    async fn download(self) -> Result<DownloadedPackedRockBytes, DownloadSrcRockError> {
        let args = self._build();
        let progress = args.progress;
        let package = args.package;
        let ext = args.ext;
        let server_url = args.server_url;
        progress.map(|p| {
            p.set_message(format!(
                "📥 Downloading {}-{}.{}",
                package.name(),
                package.version(),
                ext,
            ))
        });
        let full_rock_name = mk_packed_rock_name(package.name(), package.version(), ext);
        let url = server_url.join(&full_rock_name)?;
        let response = reqwest::Client::new().get(url.clone()).send().await?;
        let bytes = if response.status().is_success() {
            response.bytes().await
        } else {
            match args.fallback_ext {
                Some(ext) => {
                    let full_rock_name =
                        mk_packed_rock_name(package.name(), package.version(), ext);
                    let url = server_url.join(&full_rock_name)?;
                    reqwest::Client::new()
                        .get(url.clone())
                        .send()
                        .await?
                        .error_for_status()?
                        .bytes()
                        .await
                }
                None => response.error_for_status()?.bytes().await,
            }
        }?;
        Ok(DownloadedPackedRockBytes {
            name: package.name().clone(),
            version: package.version().clone(),
            bytes,
            file_name: full_rock_name,
            url,
        })
    }
}

fn mk_packed_rock_name(name: &PackageName, version: &PackageVersion, ext: &str) -> String {
    format!("{name}-{version}.{ext}")
}

pub(crate) async fn unpack_rockspec(
    rock: &DownloadedPackedRockBytes,
) -> Result<RemoteLuaRockspec, SearchAndDownloadError> {
    let cursor = Cursor::new(&rock.bytes);
    let rockspec_file_name = format!("{}-{}.rockspec", rock.name, rock.version);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|err| SearchAndDownloadError::ZipRead(rock.file_name.clone(), err))?;
    let rockspec_index = (0..zip.len())
        .find(|&i| {
            unsafe { zip.by_index(i).unwrap_unchecked() }
                .name()
                .eq(&rockspec_file_name)
        })
        .ok_or(SearchAndDownloadError::RockspecNotFoundInPackedRock(
            rockspec_file_name,
        ))?;
    let mut rockspec_file = zip
        .by_index(rockspec_index)
        .map_err(|err| SearchAndDownloadError::ZipExtract(rock.file_name.clone(), err))?;
    let mut content = String::new();
    rockspec_file.read_to_string(&mut content)?;
    let rockspec = RemoteLuaRockspec::new(&content)?;
    Ok(rockspec)
}
