use reqwest::header::ToStrError;
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;
use std::time::SystemTime;
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::{fs, io};
use url::Url;
use zip::ZipArchive;

use crate::config::{Config, LuaVersion, LuaVersionUnset};
use crate::progress::{Progress, ProgressBar};

#[derive(Error, Debug)]
pub enum ManifestFromServerError {
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

pub(super) async fn get_manifest(
    url: Url,
    manifest_version: String,
    target: &Path,
    client: &Client,
) -> Result<String, ManifestFromServerError> {
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
            .map_err(|err| ManifestFromServerError::ZipRead(url.clone(), err))?;

        let temp = tempfile::tempdir()?;

        archive
            .extract_unwrapped_root_dir(&temp, zip::read::root_dir_common_filter)
            .map_err(|err| ManifestFromServerError::ZipExtract(url.clone(), err))?;

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
pub(crate) async fn manifest_from_cache_or_server(
    server_url: &Url,
    config: &Config,
    bar: &Progress<ProgressBar>,
) -> Result<String, ManifestFromServerError> {
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
) -> Result<String, ManifestFromServerError> {
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
) -> Result<Url, ManifestFromServerError> {
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

/// Given a URL to a zip file, create a URL to the same file without the .zip extension
fn fallback_unzipped_url(url: &Url) -> Result<Url, url::ParseError> {
    url.to_string().trim_end_matches(".zip").parse()
}

#[cfg(test)]
mod tests {
    use httptest::{matchers::request, responders::status_code, Expectation, Server};
    use serial_test::serial;

    use crate::{config::ConfigBuilder, progress::MultiProgress};

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
}
