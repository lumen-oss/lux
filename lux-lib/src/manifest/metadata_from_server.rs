use miette::Diagnostic;
use reqwest::header::ToStrError;
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;
use std::time::SystemTime;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::fs;
use tracing::span;
use url::Url;
use zip::ZipArchive;

use crate::config::Config;
use crate::lua_version::{LuaVersion, LuaVersionUnset};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum ManifestFromServerError {
    #[error("IO error occured while fetching the manifest")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error("failed to pull manifest:\n{0}")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] reqwest::Error),
    #[error("failed to parse manifest:\n{0}")]
    #[diagnostic(help(
        r#"the server returned a manifest that is not valid UTF-8.
check your network connection and server configuration.
if you are using a custom server, make sure it returns correctly formatted manifests.
"#
    ))]
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
    #[diagnostic(transparent)]
    LuaVersion(#[from] LuaVersionUnset),
}

#[tracing::instrument(level = "trace", skip(client))]
pub(super) async fn get_manifest(
    url: Url,
    manifest_version: String,
    target: &Path,
    client: &Client,
) -> Result<String, ManifestFromServerError> {
    let response = client.get(url.clone()).send().await?;
    if response.status().is_client_error() {
        let fallback_url = fallback_unzipped_url(&url)?;
        let manifest_bytes = client
            .get(fallback_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let manifest = String::from_utf8(manifest_bytes.to_vec())?;
        fs::tokio::write(&target, &manifest).await?;
        Ok(manifest)
    } else {
        let manifest_bytes = response.error_for_status()?.bytes().await?;
        let mut archive = ZipArchive::new(std::io::Cursor::new(manifest_bytes))
            .map_err(|err| ManifestFromServerError::ZipRead(url.clone(), err))?;

        let temp = fs::tempfile::tempdir()?;

        archive
            .extract_unwrapped_root_dir(&temp, zip::read::root_dir_common_filter)
            .map_err(|err| ManifestFromServerError::ZipExtract(url.clone(), err))?;

        let extracted_manifest_path = temp.path().join(format!("manifest-{manifest_version}"));
        let mut extracted_manifest = fs::tokio::open(&extracted_manifest_path).await?;
        let mut target_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(target)
            .await
            .map_err(|source| fs::FsError::FileOpen {
                path: target.to_path_buf(),
                source,
            })?;

        io::copy(&mut extracted_manifest, &mut target_file)
            .await
            .map_err(|source| fs::FsError::Copy {
                from: extracted_manifest_path.to_path_buf(),
                to: target.to_path_buf(),
                source,
            })?;

        let mut manifest = String::new();

        target_file.seek(io::SeekFrom::Start(0)).await?;
        target_file.read_to_string(&mut manifest).await?;

        Ok(manifest)
    }
}

#[tracing::instrument(level = "trace", skip(config))]
pub(crate) async fn manifest_from_cache_or_server(
    server_url: &Url,
    config: &Config,
) -> Result<String, ManifestFromServerError> {
    let manifest_version = LuaVersion::from(config)?.version_compatibility_str();
    let url = mk_manifest_url(server_url, &manifest_version, config)?;
    let span = span!(
        tracing::Level::INFO,
        "Downloading manifest",
        url = url.to_string(),
    );
    let _enter = span.enter();

    let cache = mk_manifest_cache(&url, config).await?;

    #[cfg(not(test))]
    let client = crate::reqwest::new_https_client(config)?;

    #[cfg(test)]
    let client = crate::reqwest::new_http_client(config)?;

    if let Ok(metadata) = fs::tokio::metadata(&cache).await {
        let last_modified_local: SystemTime = metadata.modified()?;

        let response = match client.head(url.clone()).send().await? {
            response if response.status().is_client_error() => {
                let url = fallback_unzipped_url(&url)?;
                client.head(url).send().await?.error_for_status()?
            }
            response => response.error_for_status()?,
        };

        if let Some(last_modified_header) = response.headers().get("Last-Modified") {
            let server_last_modified = httpdate::parse_http_date(last_modified_header.to_str()?)?;

            if server_last_modified > last_modified_local {
                return get_manifest(url, manifest_version.clone(), &cache, &client).await;
            }

            return Ok(fs::tokio::read_to_string(&cache).await?);
        }
    }

    get_manifest(url, manifest_version.clone(), &cache, &client).await
}

#[tracing::instrument(level = "trace", skip(config))]
pub(crate) async fn manifest_from_server_only(
    server_url: &Url,
    config: &Config,
) -> Result<String, ManifestFromServerError> {
    let manifest_version = LuaVersion::from(config)?.version_compatibility_str();
    let url = mk_manifest_url(server_url, &manifest_version, config)?;

    let span = span!(
        tracing::Level::INFO,
        "Downloading manifest",
        url = url.to_string(),
    );
    let _enter = span.enter();

    let cache = mk_manifest_cache(&url, config).await?;
    let client = crate::reqwest::new_https_client(config)?;
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

async fn mk_manifest_cache(url: &Url, config: &Config) -> Result<PathBuf, fs::FsError> {
    let cache = config.cache_dir().join(
        // Convert the url to a directory name so we don't create too many subdirectories
        url.to_string()
            .replace(&[':', '*', '?', '"', '<', '>', '|', '/', '\\'][..], "_")
            .trim_end_matches(".zip"),
    );
    // Ensure all intermediate directories for the cache file are created (e.g. `~/.cache/lux/manifest`)
    if let Some(cache_parent_dir) = cache.parent() {
        fs::tokio::create_dir_all(cache_parent_dir).await?;
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

    use crate::{config::ConfigBuilder, fs};

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
        let mut url_str = server.url_str("/");
        url_str.pop(); // Remove trailing "/"
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir))
            .lua_version(Some(LuaVersion::LuaJIT))
            .no_progress(Some(true))
            .build()
            .unwrap();
        manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    pub async fn get_manifest_for_5_1() {
        let cache_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let server = start_test_server("manifest-5.1".into());
        let mut url_str = server.url_str("/");
        url_str.pop(); // Remove trailing "/"

        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir))
            .lua_version(Some(LuaVersion::Lua51))
            .no_progress(Some(true))
            .build()
            .unwrap();

        manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    pub async fn get_cached_manifest() {
        let server = start_test_server("manifest-5.1".into());
        let mut url_str = server.url_str("/");
        url_str.pop(); // Remove trailing "/"
        let manifest_content = fs::sync::read_to_string(
            format!("{}/resources/test/manifest-5.1", env!("CARGO_MANIFEST_DIR")).as_str(),
        )
        .unwrap();
        let cache_dir = assert_fs::TempDir::new().unwrap();
        let cache = cache_dir.join("manifest-5.1");
        fs::tokio::write(&cache, &manifest_content).await.unwrap();
        let _metadata = fs::tokio::metadata(&cache).await.unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir.to_path_buf()))
            .lua_version(Some(LuaVersion::Lua51))
            .no_progress(Some(true))
            .build()
            .unwrap();
        let result = manifest_from_cache_or_server(&Url::parse(&url_str).unwrap(), &config)
            .await
            .unwrap();
        assert_eq!(result, manifest_content);
    }
}
