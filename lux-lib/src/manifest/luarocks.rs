use itertools::Itertools;
use mlua::{Lua, LuaSerdeExt};
use reqwest::{header::ToStrError, Client};
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;
use std::time::SystemTime;
use std::{cmp::Ordering, collections::HashMap};
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::{fs, io};
use url::Url;
use zip::ZipArchive;

use crate::config::LuaVersionUnset;
use crate::manifest::ManifestMetadata;
use crate::package::{RemotePackageType, RemotePackageTypeFilterSpec};
use crate::progress::{Progress, ProgressBar};
use crate::{
    config::{Config, LuaVersion},
    package::{PackageName, PackageReq, PackageSpec, PackageVersion, RemotePackage},
    remote_package_source::RemotePackageSource,
};

#[derive(Error, Debug)]
pub enum ManifestFromLuarocksServerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("failed to pull manifest:\n{0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to parse manifest:\n{0}")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("invalidate date received from server:\n{0}")]
    InvalidDate(#[from] httpdate::Error),
    #[error("non-ASCII characters returned in response header:\n{0}")]
    InvalidHeader(#[from] ToStrError),
    #[error("error parsing manifest URL:\n{0}")]
    Url(#[from] url::ParseError),
    #[error("failed to read manifest archive {0}:\n{1}")]
    ZipRead(Url, zip::result::ZipError),
    #[error("failed to unzip manifest file {0}:\n{1}")]
    ZipExtract(Url, zip::result::ZipError),
    #[error(transparent)]
    LuaVersion(#[from] LuaVersionUnset),
}

async fn get_manifest(
    url: Url,
    manifest_version: String,
    target: &Path,
    client: &Client,
) -> Result<String, ManifestFromLuarocksServerError> {
    let response = client.get(url.clone()).send().await?;
    if response.status().is_client_error() {
        let url = fallback_unzipped_url(&url)?;
        let manifest_bytes = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let manifest = String::from_utf8(manifest_bytes.to_vec())?;
        tokio::fs::write(&target, &manifest).await?;
        Ok(manifest)
    } else {
        let manifest_bytes = response.error_for_status()?.bytes().await?;
        let mut archive = ZipArchive::new(std::io::Cursor::new(manifest_bytes))
            .map_err(|err| ManifestFromLuarocksServerError::ZipRead(url.clone(), err))?;

        let temp = tempfile::tempdir()?;

        archive
            .extract_unwrapped_root_dir(&temp, zip::read::root_dir_common_filter)
            .map_err(|err| ManifestFromLuarocksServerError::ZipExtract(url.clone(), err))?;

        let mut extracted_manifest =
            File::open(temp.path().join(format!("manifest-{manifest_version}"))).await?;
        let mut target = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(target)
            .await?;

        io::copy(&mut extracted_manifest, &mut target).await?;

        let mut manifest = String::new();

        target.seek(io::SeekFrom::Start(0)).await?;
        target.read_to_string(&mut manifest).await?;

        Ok(manifest)
    }
}

/// Look up the manifest from a cache, or get the manifest from the server
/// if the cache doesn't exist or is outdated.
async fn manifest_from_cache_or_server(
    server_url: &Url,
    config: &Config,
    bar: &Progress<ProgressBar>,
) -> Result<String, ManifestFromLuarocksServerError> {
    let manifest_version = LuaVersion::from(config)?.version_compatibility_str();
    let url = mk_manifest_url(server_url, &manifest_version, config)?;

    // Stores a path to the manifest cache (this allows us to operate on a manifest without
    // needing to pull it from the luarocks servers each time).
    let cache = mk_manifest_cache(&url, config).await?;

    let client = Client::new();

    // Read the metadata of the local cache and attempt to get the last modified date.
    if let Ok(metadata) = fs::metadata(&cache).await {
        let last_modified_local: SystemTime = metadata.modified()?;

        // Ask the server for the last modified date of its manifest.
        let response = match client.head(url.clone()).send().await? {
            response if response.status().is_client_error() => {
                let url = fallback_unzipped_url(&url)?;
                client.head(url).send().await?.error_for_status()?
            }
            response => response.error_for_status()?,
        };

        if let Some(last_modified_header) = response.headers().get("Last-Modified") {
            let server_last_modified = httpdate::parse_http_date(last_modified_header.to_str()?)?;

            // If the server's version of the manifest is newer than ours then update out manifest.
            if server_last_modified > last_modified_local {
                // Since we only pulled in the headers previously we must now request the entire
                // manifest from scratch.
                bar.map(|bar| {
                    bar.set_message(format!("📥 Downloading updated manifest from {}", &url))
                });

                return get_manifest(url, manifest_version.clone(), &cache, &client).await;
            }

            // Else return the cached manifest.
            return Ok(fs::read_to_string(&cache).await?);
        }
    }

    // If our cache file does not exist then pull the whole manifest.
    // TODO(#337): switch to something that can report progress
    bar.map(|bar| bar.set_message(format!("📥 Downloading manifest from {}", &url)));

    get_manifest(url, manifest_version.clone(), &cache, &client).await
}

/// Get the manifest from the server, ignoring the cache.
/// This still populates the cache.
pub(crate) async fn manifest_from_server_only(
    server_url: &Url,
    config: &Config,
    bar: &Progress<ProgressBar>,
) -> Result<String, ManifestFromLuarocksServerError> {
    let manifest_version = LuaVersion::from(config)?.version_compatibility_str();
    let url = mk_manifest_url(server_url, &manifest_version, config)?;
    let cache = mk_manifest_cache(&url, config).await?;
    let client = Client::new();
    bar.map(|bar| bar.set_message(format!("📥 Downloading manifest from {}", &url)));
    get_manifest(url, manifest_version.clone(), &cache, &client).await
}

fn mk_manifest_url(
    server_url: &Url,
    manifest_version: &str,
    config: &Config,
) -> Result<Url, ManifestFromLuarocksServerError> {
    let manifest_filename = format!("manifest-{manifest_version}.zip");
    let url = match config.namespace() {
        Some(namespace) => server_url
            .join(&format!("manifests/{namespace}/"))?
            .join(&manifest_filename)?,
        None => server_url.join(&manifest_filename)?,
    };
    Ok(url)
}

async fn mk_manifest_cache(url: &Url, config: &Config) -> io::Result<PathBuf> {
    let cache = config.cache_dir().join(
        // Convert the url to a directory name so we don't create too many subdirectories
        url.to_string()
            .replace(&[':', '*', '?', '"', '<', '>', '|', '/', '\\'][..], "_")
            .trim_end_matches(".zip"),
    );
    // Ensure all intermediate directories for the cache file are created (e.g. `~/.cache/lux/manifest`)
    if let Some(cache_parent_dir) = cache.parent() {
        fs::create_dir_all(cache_parent_dir).await?;
    }
    Ok(cache)
}

#[derive(Clone, Debug)]
pub(crate) struct LuarocksManifestMetadata {
    pub repository: HashMap<PackageName, HashMap<PackageVersion, Vec<RemotePackageType>>>,
}

impl<'de> serde::Deserialize<'de> for LuarocksManifestMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let intermediate = IntermediateManifest::deserialize(deserializer)?;
        Ok(Self::from_intermediate(intermediate))
    }
}

#[derive(Error, Debug)]
#[error("failed to parse Lua manifest:\n{0}")]
pub struct ManifestLuaError(#[from] mlua::Error);

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error(transparent)]
    Lua(#[from] ManifestLuaError),
    #[error("failed to fetch manifest from server:\n{0}")]
    Server(#[from] ManifestFromLuarocksServerError),
}

impl LuarocksManifestMetadata {
    pub fn new(manifest: &String) -> Result<Self, ManifestLuaError> {
        let lua = Lua::new();

        lua.load(manifest).exec()?;

        let intermediate = IntermediateManifest {
            repository: lua.from_value(lua.globals().get("repository")?)?,
        };
        let manifest = Self::from_intermediate(intermediate);

        Ok(manifest)
    }

    pub fn has_rock(&self, rock_name: &PackageName) -> bool {
        self.repository.contains_key(rock_name)
    }

    pub fn latest_match(
        &self,
        lua_package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<(PackageSpec, RemotePackageType)> {
        let filter = filter.unwrap_or_default();
        if !self.has_rock(lua_package_req.name()) {
            return None;
        }

        let (version, rock_type) = self.repository[lua_package_req.name()]
            .iter()
            .filter(|(version, _)| lua_package_req.version_req().matches(version))
            .flat_map(|(version, rock_types)| {
                rock_types.iter().filter_map(move |rock_type| {
                    let include = match rock_type {
                        RemotePackageType::Rockspec => filter.rockspec,
                        RemotePackageType::Src => filter.src,
                        RemotePackageType::Binary => filter.binary,
                    };
                    if include {
                        Some((version, rock_type))
                    } else {
                        None
                    }
                })
            })
            .max_by(
                |(version_a, type_a), (version_b, type_b)| match version_a.cmp(version_b) {
                    Ordering::Equal => type_a.cmp(type_b),
                    ordering => ordering,
                },
            )?;

        Some((
            PackageSpec::new(lua_package_req.name().clone(), version.clone()),
            rock_type.clone(),
        ))
    }

    /// Construct a `ManifestMetadata` from an intermediate representation,
    /// silently skipping entries for versions we don't know how to parse.
    fn from_intermediate(intermediate: IntermediateManifest) -> Self {
        let repository = intermediate
            .repository
            .into_iter()
            .map(|(name, package_map)| {
                (
                    name,
                    package_map
                        .into_iter()
                        .filter_map(|(version_str, entries)| {
                            let version = PackageVersion::parse(version_str.as_str()).ok()?;
                            let entries = entries
                                .into_iter()
                                .filter_map(|entry| RemotePackageType::try_from(entry).ok())
                                .collect_vec();
                            Some((version, entries))
                        })
                        .collect(),
                )
            })
            .collect();
        Self { repository }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LuarocksManifest {
    server_url: Url,
    metadata: LuarocksManifestMetadata,
}

impl LuarocksManifest {
    pub fn new(server_url: Url, metadata: LuarocksManifestMetadata) -> Self {
        Self {
            server_url,
            metadata,
        }
    }

    pub async fn from_config(
        server_url: Url,
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, ManifestError> {
        let content = manifest_from_cache_or_server(&server_url, config, progress).await?;
        match LuarocksManifestMetadata::new(&content) {
            Ok(metadata) => Ok(Self::new(server_url, metadata)),
            Err(_) => {
                let manifest = manifest_from_server_only(&server_url, config, progress).await?;
                Ok(Self::new(server_url, LuarocksManifestMetadata::new(&manifest)?))
            }
        }
    }

    pub fn metadata(&self) -> &LuarocksManifestMetadata {
        &self.metadata
    }
}

impl ManifestMetadata for LuarocksManifest {
    fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage> {
        match self.metadata().latest_match(package_req, filter) {
            None => None,
            Some((package, package_type)) => {
                let remote_source = match package_type {
                    RemotePackageType::Rockspec => {
                        RemotePackageSource::LuarocksRockspec(self.server_url().clone())
                    }
                    RemotePackageType::Src => {
                        RemotePackageSource::LuarocksSrcRock(self.server_url().clone())
                    }
                    RemotePackageType::Binary => {
                        RemotePackageSource::LuarocksBinaryRock(self.server_url().clone())
                    }
                };
                Some(RemotePackage::new(package, remote_source, None))
            }
        }
    }

    fn server_url(&self) -> &Url {
        &self.server_url
    }
}

struct UnsupportedArchitectureError;

impl TryFrom<ManifestRockEntry> for RemotePackageType {
    type Error = UnsupportedArchitectureError;
    fn try_from(
        ManifestRockEntry { arch }: ManifestRockEntry,
    ) -> Result<Self, UnsupportedArchitectureError> {
        match arch.as_str() {
            "rockspec" => Ok(RemotePackageType::Rockspec),
            "src" => Ok(RemotePackageType::Src),
            "all" => Ok(RemotePackageType::Binary),
            arch if arch == crate::luarocks::current_platform_luarocks_identifier() => {
                Ok(RemotePackageType::Binary)
            }
            _ => Err(UnsupportedArchitectureError),
        }
    }
}

#[derive(Clone, serde::Deserialize)]
struct ManifestRockEntry {
    /// e.g. "linux-x86_64", "rockspec", "src", ...
    pub arch: String,
}

/// Intermediate implementation for deserializing
#[derive(serde::Deserialize)]
struct IntermediateManifest {
    /// The key of each package's HashMap is the version string
    repository: HashMap<PackageName, HashMap<String, Vec<ManifestRockEntry>>>,
}

/// Given a URL to a zip file, create a URL to the same file without the .zip extension
fn fallback_unzipped_url(url: &Url) -> Result<Url, url::ParseError> {
    url.to_string().trim_end_matches(".zip").parse()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use httptest::{matchers::request, responders::status_code, Expectation, Server};
    use serial_test::serial;

    use crate::{config::ConfigBuilder, package::PackageReq, progress::MultiProgress};

    use super::*;

    fn start_test_server(manifest_name: String) -> Server {
        let server = Server::run();
        let manifest_path = format!("/{manifest_name}");
        server.expect(
            Expectation::matching(request::path(manifest_path + ".zip"))
                .times(1..)
                .respond_with(
                    status_code(200)
                        .append_header("Last-Modified", "Sat, 20 Jan 2024 13:14:12 GMT")
                        .body(
                            std::fs::read(
                                format!(
                                    "{}/resources/test/manifest-5.1.zip",
                                    env!("CARGO_MANIFEST_DIR")
                                )
                                .as_str(),
                            )
                            .unwrap(),
                        ),
                ),
        );
        server
    }

    #[tokio::test]
    #[serial]
    pub async fn get_manifest_luajit() {
        let cache_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let server = start_test_server("manifest-5.1".into());
        let mut url_str = server.url_str(""); // Remove trailing "/"
        url_str.pop();
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir))
            .lua_version(Some(crate::config::LuaVersion::LuaJIT))
            .no_progress(Some(true))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config, &bar)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    pub async fn get_manifest_for_5_1() {
        let cache_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let server = start_test_server("manifest-5.1".into());
        let mut url_str = server.url_str(""); // Remove trailing "/"
        url_str.pop();

        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir))
            .lua_version(Some(crate::config::LuaVersion::Lua51))
            .no_progress(Some(true))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);

        manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config, &bar)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    pub async fn get_cached_manifest() {
        let server = start_test_server("manifest-5.1".into());
        let mut url_str = server.url_str(""); // Remove trailing "/"
        url_str.pop();
        let manifest_content = std::fs::read_to_string(
            format!("{}/resources/test/manifest-5.1", env!("CARGO_MANIFEST_DIR")).as_str(),
        )
        .unwrap();
        let cache_dir = assert_fs::TempDir::new().unwrap();
        let cache = cache_dir.join("manifest-5.1");
        fs::write(&cache, &manifest_content).await.unwrap();
        let _metadata = fs::metadata(&cache).await.unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir.to_path_buf()))
            .lua_version(Some(crate::config::LuaVersion::Lua51))
            .no_progress(Some(true))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let result = manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config, &bar)
            .await
            .unwrap();
        assert_eq!(result, manifest_content);
    }

    #[tokio::test]
    pub async fn parse_metadata_from_empty_manifest() {
        let manifest = "
            commands = {}\n
            modules = {}\n
            repository = {}\n
            "
        .to_string();
        LuarocksManifestMetadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn parse_metadata_from_test_manifest() {
        let mut test_manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        test_manifest_path.push("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        LuarocksManifestMetadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn latest_match_regression() {
        let mut test_manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        test_manifest_path.push("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        let metadata = LuarocksManifestMetadata::new(&manifest).unwrap();

        let package_req: PackageReq = "30log > 1.3.0".parse().unwrap();
        assert!(metadata.latest_match(&package_req, None).is_none());
    }
}
