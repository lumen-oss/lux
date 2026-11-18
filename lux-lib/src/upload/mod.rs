use crate::package;
use std::{env, io, path::PathBuf};

use crate::fs;
use crate::luarocks;
use crate::operations::{download, pack, sign};
use crate::package::db::PackageDB;
use crate::package::SpecRevIterator;
use crate::rockspec;
use crate::rockspec::Rockspec;
use crate::{config::Config, project::Project};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use serde::Deserialize;
use tempfile::TempDir;
use thiserror::Error;
use url::Url;

const TFA_TOKEN_HEADER: &str = "X-TFA-Token";

pub mod check;

/// WARNING: This function is unsafe,
/// because it adds the unmasked API key to the URL.
/// When using URLs created by this function,
/// pay attention not to leak the API key in errors.
pub(crate) unsafe fn url_for_method(
    server_url: &Url,
    api_key: &ApiKey,
    endpoint: &str,
) -> Result<Url, url::ParseError> {
    server_url
        .join("api/1/")?
        .join(&format!("{}/", api_key.get()))?
        .join(endpoint)
}

/// Whether to perform the upload or only report what would be uploaded.
#[derive(Clone, Copy, Debug, Default)]
pub enum Mode {
    #[default]
    Upload,
    DryRun,
}

/// A rocks package uploader, providing fine-grained control
/// over how a package should be uploaded.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct ProjectUpload<'a> {
    project: &'a Project,
    api_key: Option<ApiKey>,
    tfa_code: Option<String>,
    #[cfg(feature = "gpgme")]
    sign_protocol: sign::Protocol,
    config: &'a Config,
    package_db: &'a PackageDB,
    #[builder(default)]
    mode: Mode,
    #[builder(default)]
    pack_source_rock: bool,
}

impl<State> ProjectUploadBuilder<'_, State>
where
    State: project_upload_builder::State + project_upload_builder::IsComplete,
{
    /// Upload a package to a luarocks server.
    pub async fn upload_to_luarocks(self) -> Result<PreparedRockspec, Error> {
        let args = self._build();
        upload_from_project(args).await
    }
}

/// An uploader for an existing `.rockspec` file.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct RockspecUpload<'a> {
    rockspec: &'a rockspec::lua::Remote,
    api_key: Option<ApiKey>,
    tfa_code: Option<String>,
    #[cfg(feature = "gpgme")]
    sign_protocol: sign::Protocol,
    config: &'a Config,
    #[builder(default)]
    mode: Mode,
    #[builder(default)]
    pack_source_rock: bool,
}

impl<State> RockspecUploadBuilder<'_, State>
where
    State: rockspec_upload_builder::State + rockspec_upload_builder::IsComplete,
{
    /// Upload a rockspec to a luarocks server.
    pub async fn upload_to_luarocks(self) -> Result<PreparedRockspec, Error> {
        let args = self._build();
        upload_from_rockspec(args).await
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(transparent)]
pub enum Error {
    #[error("error parsing upload URL")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("HTPP request error while uploading")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] crate::reqwest::Error),
    #[error("server '{0}' responded with error status: '{1}'")]
    #[diagnostic(help("the server may be temporarily unavailable"))]
    Server(Url, StatusCode),
    #[error("client error when requesting {0}:\n{1}")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Client(Url, String),
    #[diagnostic(transparent)]
    RockCheck(#[from] check::rock::Error),
    #[error("a package with the same rockspec content already exists on the server: '{0}'")]
    #[diagnostic(help("try uploading a rockspec with a different version"))]
    RockExists(Url),
    #[error("unable to read rockspec")]
    #[diagnostic(help("check that the rockspec exists and is readable"))]
    RockspecRead(#[from] std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    SourcePack(#[from] pack::SourceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Sign(#[from] sign::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ToolCheck(#[from] check::tool::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    UserCheck(#[from] check::user::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ApiKeyUnspecified(#[from] ApiKeyUnspecified),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ValidationError(#[from] rockspec::toml::Error),
    #[error("unsupported version: '{0}'")]
    #[diagnostic(help("Lux can upload packages with a SemVer version, 'dev' or 'scm'"))]
    UnsupportedVersion(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Rockspec(#[from] rockspec::Error),
    #[error("the maximum supported number of rockspec revisions per version has been exceeded")]
    #[diagnostic(help("bump the version to a version that has not yet been published"))]
    MaxSpecRevsExceeded,
    #[error("the rockspec was published but the source rock upload failed")]
    #[diagnostic(help("bump the package revision and upload again to publish the source rock"))]
    SourceRockUpload(#[source] Box<Error>),
    #[error("rock already exists on server. Error downloading existing rockspec")]
    #[diagnostic(forward(0))]
    SearchAndDownload(#[from] Box<download::Error>),
    #[error("error computing rockspec hash")]
    Hash(io::Error),
    #[error("the 2FA code '{0}' was rejected by the server: {1}")]
    #[diagnostic(help("it may have expired; try again with a new code."))]
    TfaCodeRejected(String, String),
}

impl From<download::Error> for Error {
    fn from(source: download::Error) -> Self {
        Self::SearchAndDownload(Box::new(source))
    }
}

pub struct ApiKey(String);

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error("no API key provided")]
#[diagnostic(help("please set the $LUX_API_KEY environment variable"))]
pub struct ApiKeyUnspecified;

impl ApiKey {
    /// Retrieves the rocks API key from the `$LUX_API_KEY` environment
    /// variable and seals it in this struct.
    pub fn new() -> Result<Self, ApiKeyUnspecified> {
        Ok(Self(
            env::var("LUX_API_KEY").map_err(|_| ApiKeyUnspecified)?,
        ))
    }

    /// Creates an API key from a [`String`].
    ///
    /// # Safety
    ///
    /// This struct is designed to be sealed without a [`Display`](std::fmt::Display) implementation
    /// so that it can never accidentally be printed.
    ///
    /// Ensure that you do not do anything else with the API key string prior to sealing it in this
    /// struct.
    pub fn from(str: &str) -> Self {
        Self(str.to_string())
    }

    /// Retrieves the underlying API key as a [`String`].
    ///
    /// # Safety
    ///
    /// Strings may accidentally be printed as part of its [`Display`](std::fmt::Display)
    /// implementation. Ensure that you never pass this variable somewhere it may be displayed.
    pub unsafe fn get(&self) -> &str {
        &self.0
    }
}

/// 2FA token.
/// This struct is designed to be sealed without a [`Display`](std::fmt::Display) implementation
/// so that it can never accidentally be printed.
struct TfaToken(String);

impl TfaToken {
    /// Retrieves the underlying 2FA token as a [`String`].
    ///
    /// # Safety
    ///
    /// Strings may accidentally be printed as part of its [`Display`](std::fmt::Display)
    /// implementation. Ensure that you never pass this variable somewhere it may be displayed.
    unsafe fn get(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TfaToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self)
    }
}

#[derive(Deserialize)]
struct LuarocksTfaVerificationSuccess {
    tfa_token: TfaToken,
}

/// Models the response received from luarocks.org
#[derive(Deserialize)]
#[serde(untagged)]
enum LuarocksTfaVerificationResponse {
    Success(LuarocksTfaVerificationSuccess),
    Failure(LuarocksErrorResponse),
}

#[derive(Deserialize)]
struct LuarocksErrorResponse {
    errors: Vec<String>,
}

struct Context<'a> {
    config: &'a Config,
    client: &'a reqwest::Client,
    api_key: &'a ApiKey,
}

struct PackedSourceRock {
    _dir: TempDir,
    path: PathBuf,
}

#[tracing::instrument(
    name = "Uploading",
    level = "info",
    skip_all,
    fields(project = args.project.toml().package().to_string()),
)]
async fn upload_from_project(args: ProjectUpload<'_>) -> Result<PreparedRockspec, Error> {
    let project = args.project;
    let api_key = args.api_key.unwrap_or(ApiKey::new()?);
    let config = args.config;
    let package_db = args.package_db;

    let client = config.https_client()?;

    check::tool::ensure_version(client, config.server()).await?;
    check::user::ensure(client, &api_key, config.server()).await?;

    let (rockspec, rockspec_content) =
        helpers::generate_rockspec(project, client, &api_key, config, package_db).await?;

    #[cfg(feature = "gpgme")]
    let protocol = args.sign_protocol;
    #[cfg(not(feature = "gpgme"))]
    let protocol = sign::Protocol::None;

    let context = Context {
        config,
        client,
        api_key: &api_key,
    };
    let source_rock = if args.pack_source_rock {
        prepare_source_rock(
            &rockspec,
            &rockspec_content,
            config,
            args.mode,
            protocol.clone(),
        )
        .await?
    } else {
        None
    };
    upload_prepared(
        &context,
        &rockspec,
        rockspec_content,
        protocol,
        args.tfa_code,
        args.mode,
        source_rock,
    )
    .await
}

#[tracing::instrument(
    name = "Uploading",
    level = "info",
    skip_all,
    fields(package = args.rockspec.package().to_string()),
)]
async fn upload_from_rockspec(args: RockspecUpload<'_>) -> Result<PreparedRockspec, Error> {
    let rockspec = args.rockspec;
    let api_key = args.api_key.unwrap_or(ApiKey::new()?);
    #[cfg(feature = "gpgme")]
    let protocol = args.sign_protocol;
    #[cfg(not(feature = "gpgme"))]
    let protocol = sign::Protocol::None;
    let config = args.config;

    let client = config.https_client()?;

    check::tool::ensure_version(client, config.server()).await?;
    check::user::ensure(client, &api_key, config.server()).await?;

    if let package::Version::StringVer(ver) = rockspec.version() {
        return Err(Error::UnsupportedVersion(ver.to_string()));
    }

    if check::rock::exists(
        client,
        &api_key,
        rockspec.package(),
        rockspec.version(),
        config.server(),
    )
    .await?
    {
        return Err(Error::RockExists(config.server().clone()));
    }

    let rockspec_content = rockspec.to_lua_remote_rockspec_string()?;
    let context = Context {
        config,
        client,
        api_key: &api_key,
    };
    let source_rock = if args.pack_source_rock {
        prepare_source_rock(
            rockspec,
            &rockspec_content,
            config,
            args.mode,
            protocol.clone(),
        )
        .await?
    } else {
        None
    };
    upload_prepared(
        &context,
        rockspec,
        rockspec_content,
        protocol,
        args.tfa_code,
        args.mode,
        source_rock,
    )
    .await
}

struct PreparedSourceRock {
    file_name: String,
    payload: Option<SourceRockPayload>,
}

struct SourceRockPayload {
    bytes: Vec<u8>,
    signature: Option<sign::Signature>,
}

async fn prepare_source_rock<R: Rockspec>(
    rockspec: &R,
    rockspec_content: &str,
    config: &Config,
    mode: Mode,
    protocol: sign::Protocol,
) -> Result<Option<PreparedSourceRock>, Error> {
    if matches!(rockspec.version(), package::Version::DevVer(_)) {
        tracing::warn!("skipping source rock for dev version");
        return Ok(None);
    }
    let file_name = luarocks::rock_file_name(rockspec.package(), rockspec.version(), "src.rock");
    if matches!(mode, Mode::DryRun) {
        return Ok(Some(PreparedSourceRock {
            file_name,
            payload: None,
        }));
    }
    let packed = pack_source_rock(rockspec, rockspec_content, config).await?;
    let bytes = fs::tokio::read(&packed.path).await?;
    let signature = sign::file(&bytes, protocol)?;
    Ok(Some(PreparedSourceRock {
        file_name,
        payload: Some(SourceRockPayload { bytes, signature }),
    }))
}

async fn pack_source_rock<R: Rockspec>(
    rockspec: &R,
    rockspec_content: &str,
    config: &Config,
) -> Result<PackedSourceRock, Error> {
    let dir = fs::tempfile::tempdir()?;
    let path = pack::Source::new(dir.path(), rockspec, rockspec_content, config)
        .pack()
        .await?;
    Ok(PackedSourceRock { _dir: dir, path })
}

async fn upload_prepared<R: Rockspec>(
    context: &Context<'_>,
    rockspec: &R,
    rockspec_content: String,
    protocol: sign::Protocol,
    tfa_code: Option<String>,
    mode: Mode,
    source_rock: Option<PreparedSourceRock>,
) -> Result<PreparedRockspec, Error> {
    let source_rock_name = source_rock.as_ref().map(|rock| rock.file_name.clone());
    let signed = sign::file(rockspec_content.as_bytes(), protocol)?;
    let prepared = PreparedRockspec::new(rockspec, rockspec_content, source_rock_name);
    let tfa_token = match tfa_code.as_deref() {
        Some(code) => Some(
            helpers::verify_tfa_code(
                context.client,
                context.config.server(),
                context.api_key,
                code,
            )
            .await?,
        ),
        None => None,
    };
    let version_id =
        upload_rockspec_content(context, &prepared, signed, tfa_token.as_ref(), mode).await?;
    if let Some(rock) = source_rock {
        if let Some(payload) = rock.payload {
            let Some(version_id) = version_id else {
                tracing::warn!("no version id in the response; skipping the source rock upload");
                return Ok(prepared);
            };
            upload_rock(
                context,
                version_id,
                rock.file_name,
                payload.bytes,
                payload.signature,
                tfa_token.as_ref(),
            )
            .await
            .map_err(|error| Error::SourceRockUpload(Box::new(error)))?;
        }
    }
    Ok(prepared)
}

/// The rockspec that was or would have been uploaded.
#[derive(Debug)]
pub struct PreparedRockspec {
    pub package: package::Name,
    pub version: package::Version,
    pub content: String,
    pub source_rock: Option<String>,
}

impl PreparedRockspec {
    fn new<R: Rockspec>(rockspec: &R, content: String, source_rock: Option<String>) -> Self {
        Self {
            package: rockspec.package().clone(),
            version: rockspec.version().clone(),
            content,
            source_rock,
        }
    }
}

async fn upload_rockspec_content(
    context: &Context<'_>,
    rockspec: &PreparedRockspec,
    signed: Option<sign::Signature>,
    tfa_token: Option<&TfaToken>,
    mode: Mode,
) -> Result<Option<u64>, Error> {
    if !matches!(mode, Mode::Upload) {
        return Ok(None);
    }
    let request = prepare_upload_request(
        context.config,
        context.client,
        context.api_key,
        rockspec,
        signed,
        tfa_token,
    )?;
    send_upload_request(context.config, request).await
}

fn add_signature_parts(
    multipart: Form,
    signature: sign::Signature,
    signature_part: &'static str,
    key_part: &'static str,
    signature_file: &str,
    key_file: &str,
) -> Form {
    multipart
        .part(
            signature_part,
            Part::text(signature.signature).file_name(signature_file.to_string()),
        )
        .part(
            key_part,
            Part::text(signature.public_key).file_name(key_file.to_string()),
        )
}

fn prepare_upload_request(
    config: &Config,
    client: &reqwest::Client,
    api_key: &ApiKey,
    rockspec: &PreparedRockspec,
    signed: Option<sign::Signature>,
    tfa_token: Option<&TfaToken>,
) -> Result<reqwest::RequestBuilder, Error> {
    let part = Part::text(rockspec.content.clone())
        .file_name(format!(
            "{}-{}.rockspec",
            rockspec.package, rockspec.version
        ))
        .mime_str("application/octet-stream")?;

    let multipart = Form::new().part("rockspec_file", part);
    let multipart = match signed {
        Some(signature) => add_signature_parts(
            multipart,
            signature,
            "rockspec_sig",
            "rockspec_key",
            "project.rockspec.sig",
            "project.rockspec.pub",
        ),
        None => multipart,
    };

    let mut request = client
        .post(unsafe { url_for_method(config.server(), api_key, "upload")? })
        .multipart(multipart);

    if let Some(token) = tfa_token {
        request = request.header(TFA_TOKEN_HEADER, unsafe { token.get() });
    }

    Ok(request)
}

async fn send_upload_request(
    config: &Config,
    request: reqwest::RequestBuilder,
) -> Result<Option<u64>, Error> {
    let response = request.send().await?;
    parse_upload_response(config, response).await
}

#[derive(Deserialize)]
struct Response {
    version: Option<Version>,
}

#[derive(Deserialize)]
struct Version {
    id: u64,
}

async fn parse_upload_response(
    config: &Config,
    response: reqwest::Response,
) -> Result<Option<u64>, Error> {
    let status = response.status();
    if status.is_server_error() {
        Err(Error::Server(config.server().clone(), status))
    } else if status.is_success() {
        let body = response.text().await?;
        Ok(serde_json::from_str::<Response>(&body)
            .ok()
            .and_then(|response| response.version)
            .map(|version| version.id))
    } else {
        let response = response.json::<LuarocksErrorResponse>().await?;
        let errors = response.errors.into_iter().join("\n");
        Err(Error::Client(config.server().clone(), errors))
    }
}

async fn upload_rock(
    context: &Context<'_>,
    version_id: u64,
    file_name: String,
    bytes: Vec<u8>,
    signed: Option<sign::Signature>,
    tfa_token: Option<&TfaToken>,
) -> Result<(), Error> {
    let rock_part = Part::bytes(bytes)
        .file_name(file_name)
        .mime_str("application/octet-stream")?;
    let multipart = Form::new().part("rock_file", rock_part);
    let multipart = match signed {
        Some(signature) => add_signature_parts(
            multipart,
            signature,
            "rock_sig",
            "rock_key",
            "project.src.rock.sig",
            "project.src.rock.pub",
        ),
        None => multipart,
    };
    let endpoint = format!("upload_rock/{version_id}");
    let mut request = context
        .client
        .post(unsafe { url_for_method(context.config.server(), context.api_key, &endpoint)? })
        .multipart(multipart);
    if let Some(token) = tfa_token {
        request = request.header(TFA_TOKEN_HEADER, unsafe { token.get() });
    }
    send_upload_request(context.config, request)
        .await
        .map(|_| ())
}

mod helpers {
    use super::*;
    use crate::hash::HasIntegrity;
    use itertools::Itertools;
    use reqwest::Client;
    use rockspec::toml;
    use ssri::Integrity;
    use url::Url;

    #[tracing::instrument(level = "trace", skip(client, api_key))]
    pub(crate) async fn verify_tfa_code(
        client: &Client,
        server_url: &Url,
        api_key: &ApiKey,
        tfa_code: &str,
    ) -> Result<TfaToken, Error> {
        let response = client
            .get(unsafe { url_for_method(server_url, api_key, "verify_tfa")? })
            .query(&[("code", tfa_code)])
            .send()
            .await?;
        let status = response.status();
        if status.is_server_error() {
            Err(Error::Server(server_url.clone(), status))
        } else {
            match response.json::<LuarocksTfaVerificationResponse>().await? {
                LuarocksTfaVerificationResponse::Success(LuarocksTfaVerificationSuccess {
                    tfa_token,
                }) => Ok(tfa_token),
                LuarocksTfaVerificationResponse::Failure(LuarocksErrorResponse { errors }) => Err(
                    Error::TfaCodeRejected(tfa_code.to_string(), errors.into_iter().join("\n")),
                ),
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) async fn generate_rockspec(
        project: &Project,
        client: &Client,
        api_key: &ApiKey,
        config: &Config,
        package_db: &PackageDB,
    ) -> Result<(toml::Remote, String), Error> {
        for specrev in SpecRevIterator::new() {
            let rockspec = project.toml().into_remote(Some(specrev))?;

            let rockspec_content = rockspec.to_lua_remote_rockspec_string()?;

            if let package::Version::StringVer(ver) = rockspec.version() {
                return Err(Error::UnsupportedVersion(ver.to_string()));
            }
            if check::rock::exists(
                client,
                api_key,
                rockspec.package(),
                rockspec.version(),
                config.server(),
            )
            .await?
            {
                let package =
                    package::Spec::new(rockspec.package().clone(), rockspec.version().clone());
                let existing_rockspec = download::exact_rockspec(&package, package_db, config)
                    .await?
                    .rockspec;
                let existing_rockspec_hash = existing_rockspec.hash().await.map_err(Error::Hash)?;
                if existing_rockspec_hash
                    .matches(&Integrity::from(&rockspec_content))
                    .is_some()
                {
                    return Err(Error::RockExists(config.server().clone()));
                }
            } else {
                return Ok((rockspec, rockspec_content));
            }
        }
        Err(Error::MaxSpecRevsExceeded)
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

#[cfg(test)]
mod test {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use crate::TOOL_VERSION;
    use bytes::Bytes;

    use super::*;

    use crate::config;
    use crate::lua;
    use assert_fs::prelude::*;
    use httptest::matchers::{contains, matches, request};
    use httptest::responders::{json_encoded, status_code, Responder};
    use httptest::{Expectation, Server};
    use serial_test::serial;

    #[cfg(feature = "gpgme")]
    #[test]
    fn signature_protocols_are_accepted_by_gpgme() {
        use strum::IntoEnumIterator;

        for protocol in sign::Protocol::iter().filter(|p| p != &sign::Protocol::None) {
            let _ = gpgme::Context::from_protocol(protocol.into()).unwrap();
        }
    }

    #[test]
    fn test_deserialize_tfa_success() {
        let response_str = r#"{
    "success": true,
    "expires": 1782939987,
    "tfa_token": "dummy_token"
}
"#;
        let result = serde_json::from_str(response_str).unwrap();
        assert!(matches!(
            result,
            LuarocksTfaVerificationResponse::Success(LuarocksTfaVerificationSuccess { .. })
        ));
    }

    #[test]
    fn test_deserialize_tfa_failure() {
        let response_str = r#"{
    "errors": [
        "Invalid verification code"
    ]
}
"#;
        let result = serde_json::from_str(response_str).unwrap();
        assert!(matches!(
            result,
            LuarocksTfaVerificationResponse::Failure(LuarocksErrorResponse { .. })
        ));
    }

    #[test]
    fn test_deserialize_luarocks_error_response() {
        let response_str = r#"{
    "errors": [
        "Invalid verification code"
    ]
}
"#;
        let result = serde_json::from_str(response_str).unwrap();
        assert!(matches!(result, LuarocksErrorResponse { .. }));
    }

    const API_KEY: &str = "test-api-key";
    const TOOL_VERSION_PATH: &str = "/api/tool_version";
    const STATUS_PATH: &str = "/api/1/test-api-key/status";
    const CHECK_ROCKSPEC_PATH: &str = "/api/1/test-api-key/check_rockspec";
    const VERIFY_TFA_PATH: &str = "/api/1/test-api-key/verify_tfa";
    const UPLOAD_PATH: &str = "/api/1/test-api-key/upload";

    async fn ready(
        server: &Server,
    ) -> (
        assert_fs::TempDir,
        assert_fs::TempDir,
        Project,
        Config,
        PackageDB,
    ) {
        std::env::set_var("LUX_API_KEY", API_KEY);
        let project_dir = assert_fs::TempDir::new().unwrap();
        project_dir
            .child("lux.toml")
            .write_str(
                r#"
package = "foo"
version = "1.0.0"
lua = ">=5.1"

[source]
url = "https://example.com/foo.zip"

[build]
type = "builtin"
"#,
            )
            .unwrap();
        let project = Project::from_exact(project_dir.path()).unwrap().unwrap();

        let config_dir = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::default()
            .server(Some(Url::parse(&server.url_str("/")).unwrap()))
            .data_dir(Some(config_dir.path().to_path_buf()))
            .cache_dir(Some(config_dir.path().to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .build()
            .unwrap();
        let package_db = PackageDB::from_config(&config).await.unwrap();

        (project_dir, config_dir, project, config, package_db)
    }

    async fn ready_with_source(
        server: &Server,
    ) -> (
        assert_fs::TempDir,
        assert_fs::TempDir,
        Project,
        Config,
        PackageDB,
    ) {
        let (project_dir, config_dir, _project, config, package_db) = ready(server).await;
        let source_dir = project_dir.path().join("source");
        std::fs::create_dir_all(source_dir.join("src")).unwrap();
        std::fs::write(source_dir.join("src/foo.lua"), "return {}").unwrap();
        let project_toml = format!(
            r#"
package = "foo"
version = "1.0.0"
lua = ">=5.1"

[source]
url = "file://{}"

[build]
type = "builtin"
"#,
            source_dir.display()
        );
        std::fs::write(project_dir.path().join("lux.toml"), project_toml).unwrap();
        let project = Project::from_exact(project_dir.path()).unwrap().unwrap();
        (project_dir, config_dir, project, config, package_db)
    }

    fn expect_tool_version(server: &Server, version: &str) {
        server.expect(
            Expectation::matching(request::path(TOOL_VERSION_PATH))
                .respond_with(json_encoded(serde_json::json!({ "version": version }))),
        );
    }

    fn expect_status(server: &Server, status: u16) {
        server.expect(
            Expectation::matching(request::path(STATUS_PATH)).respond_with(status_code(status)),
        );
    }

    fn expect_rock_missing(server: &Server) {
        server.expect(
            Expectation::matching(request::path(CHECK_ROCKSPEC_PATH))
                .respond_with(json_encoded(serde_json::json!({}))),
        );
    }

    fn expect_upload(server: &Server, responder: impl Responder + 'static) {
        server.expect(Expectation::matching(request::path(UPLOAD_PATH)).respond_with(responder));
    }

    fn expect_upload_rock(server: &Server, version_id: u64, responder: impl Responder + 'static) {
        let path = format!("/api/1/test-api-key/upload_rock/{version_id}");
        server.expect(Expectation::matching(request::path(path)).respond_with(responder));
    }

    struct CaptureBody(Arc<Mutex<String>>);

    impl Responder for CaptureBody {
        fn respond<'a>(
            &mut self,
            request: &'a httptest::http::Request<Bytes>,
        ) -> Pin<Box<dyn Future<Output = httptest::http::Response<Bytes>> + Send + 'a>> {
            *self.0.lock().unwrap() = String::from_utf8_lossy(request.body()).into_owned();
            Box::pin(async {
                httptest::http::Response::builder()
                    .status(200)
                    .body(Bytes::new())
                    .unwrap()
            })
        }
    }

    async fn upload(
        project: &Project,
        config: &Config,
        package_db: &PackageDB,
        tfa_code: Option<String>,
    ) -> Result<PreparedRockspec, Error> {
        let upload = ProjectUpload::new()
            .project(project)
            .config(config)
            .api_key(ApiKey::from(API_KEY))
            .maybe_tfa_code(tfa_code)
            .package_db(package_db);
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(sign::Protocol::None);
        upload.upload_to_luarocks().await
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_to_mock_server() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        expect_upload(&server, status_code(200));

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        upload(&project, &config, &package_db, None).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_source_rock() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        expect_upload(
            &server,
            json_encoded(serde_json::json!({ "version": { "id": 123 } })),
        );
        expect_upload_rock(&server, 123, status_code(200));

        let (_project_dir, _config_dir, project, config, package_db) =
            ready_with_source(&server).await;
        let upload = ProjectUpload::new()
            .project(&project)
            .config(&config)
            .api_key(ApiKey::from(API_KEY))
            .package_db(&package_db)
            .pack_source_rock(true);
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(sign::Protocol::None);

        let prepared = upload.upload_to_luarocks().await.unwrap();
        assert!(prepared.source_rock.is_some());
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_source_rock_without_version_id() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        expect_upload(&server, status_code(200));
        server.expect(
            Expectation::matching(request::path(matches("^/api/1/test-api-key/upload_rock/")))
                .times(0)
                .respond_with(status_code(200)),
        );

        let (_project_dir, _config_dir, project, config, package_db) =
            ready_with_source(&server).await;
        let upload = ProjectUpload::new()
            .project(&project)
            .config(&config)
            .api_key(ApiKey::from(API_KEY))
            .package_db(&package_db)
            .pack_source_rock(true);
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(sign::Protocol::None);

        let prepared = upload.upload_to_luarocks().await.unwrap();
        assert!(prepared.source_rock.is_some());
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_source_rock_dry_run() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);

        let (_project_dir, _config_dir, project, config, package_db) =
            ready_with_source(&server).await;
        let upload = ProjectUpload::new()
            .project(&project)
            .config(&config)
            .api_key(ApiKey::from(API_KEY))
            .package_db(&package_db)
            .mode(Mode::DryRun)
            .pack_source_rock(true);
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(sign::Protocol::None);

        let prepared = upload.upload_to_luarocks().await.unwrap();
        assert_eq!(
            prepared.source_rock.as_deref(),
            Some("foo-1.0.0-1.src.rock")
        );
    }

    #[tokio::test]
    async fn test_upload_sends_signature_parts() {
        let server = Server::run();
        let captured = Arc::new(Mutex::new(String::new()));
        server.expect(
            Expectation::matching(request::path(UPLOAD_PATH))
                .respond_with(CaptureBody(captured.clone())),
        );
        let content = r#"
package = "foo"
version = "1.0.0-1"
source = { url = "https://example.com/foo.zip" }
build = { type = "builtin" }
"#;
        let rockspec = crate::rockspec::lua::Remote::new(content).unwrap();
        let prepared = PreparedRockspec::new(&rockspec, content.to_string(), None);
        let dir = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::default()
            .server(Some(Url::parse(&server.url_str("/")).unwrap()))
            .data_dir(Some(dir.path().to_path_buf()))
            .cache_dir(Some(dir.path().to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .build()
            .unwrap();
        let signature = sign::Signature {
            signature: "signature".to_string(),
            public_key: "public key".to_string(),
        };

        let request = prepare_upload_request(
            &config,
            &reqwest::Client::new(),
            &ApiKey::from(API_KEY),
            &prepared,
            Some(signature),
            None,
        )
        .unwrap();
        request.send().await.unwrap();

        let captured = captured.lock().unwrap();
        assert!(captured.contains("rockspec_sig"));
        assert!(captured.contains("rockspec_key"));
    }

    #[tokio::test]
    #[serial]
    async fn test_generate_rockspec_auto_specrev_increment() {
        let server = Server::run();
        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;

        for revision in 1..3 {
            server.expect(
                Expectation::matching(httptest::matchers::all_of![
                    request::path(CHECK_ROCKSPEC_PATH),
                    request::query(httptest::matchers::url_decoded(contains((
                        "version",
                        format!("1.0.0-{revision}"),
                    )))),
                ])
                .respond_with(json_encoded(serde_json::json!({
                    "module": {},
                    "version": {},
                }))),
            );
        }
        server.expect(
            Expectation::matching(httptest::matchers::all_of![
                request::path(CHECK_ROCKSPEC_PATH),
                request::query(httptest::matchers::url_decoded(contains((
                    "version", "1.0.0-3",
                )))),
            ])
            .respond_with(json_encoded(serde_json::json!({
                "module": null,
                "version": null,
            }))),
        );
        server.expect(
            Expectation::matching(request::path("/manifest-5.1.zip"))
                .respond_with(status_code(404)),
        );
        server.expect(
            Expectation::matching(request::path("/manifest-5.1")).respond_with(
                status_code(200)
                    .body(r#"repository = { foo = { ["1.0.0-2"] = { { arch = "rockspec" } } } }"#),
            ),
        );
        for revision in 1u16..=2 {
            let content = project
                .toml()
                .into_remote(Some(revision.into()))
                .unwrap()
                .to_lua_remote_rockspec_string()
                .unwrap()
                .replace("https://example.com/foo.zip", "https://example.com/old.zip");
            server.expect(
                Expectation::matching(request::path(format!("/foo-1.0.0-{revision}.rockspec")))
                    .respond_with(status_code(200).body(content)),
            );
        }

        let (rockspec, _) = helpers::generate_rockspec(
            &project,
            &reqwest::Client::new(),
            &ApiKey::from(API_KEY),
            &config,
            &package_db,
        )
        .await
        .unwrap();

        assert_eq!(rockspec.version().to_string(), "1.0.0-3");
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_forwards_tfa_token_header() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        server.expect(
            Expectation::matching(request::path(VERIFY_TFA_PATH)).respond_with(json_encoded(
                serde_json::json!({ "success": true, "tfa_token": "tfa-token-123" }),
            )),
        );
        server.expect(
            Expectation::matching(request::headers(contains(("x-tfa-token", "tfa-token-123"))))
                .respond_with(status_code(200)),
        );

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        upload(&project, &config, &package_db, Some("123456".into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_rejected_tfa_code() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        server.expect(
            Expectation::matching(request::path(VERIFY_TFA_PATH)).respond_with(json_encoded(
                serde_json::json!({ "errors": ["Invalid verification code"] }),
            )),
        );

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let err = upload(&project, &config, &package_db, Some("000000".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::TfaCodeRejected(..)));
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_dry_run_rejects_tfa_code() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        server.expect(
            Expectation::matching(request::path(VERIFY_TFA_PATH)).respond_with(json_encoded(
                serde_json::json!({ "errors": ["Invalid verification code"] }),
            )),
        );

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let upload = ProjectUpload::new()
            .project(&project)
            .config(&config)
            .api_key(ApiKey::from(API_KEY))
            .package_db(&package_db)
            .mode(Mode::DryRun)
            .maybe_tfa_code(Some("000000".to_string()));
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(sign::Protocol::None);

        let err = upload.upload_to_luarocks().await.unwrap_err();
        assert!(matches!(err, Error::TfaCodeRejected(..)));
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_server_error() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        expect_upload(&server, status_code(500));

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let err = upload(&project, &config, &package_db, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Server(_, StatusCode::INTERNAL_SERVER_ERROR)
        ));
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_client_error() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 200);
        expect_rock_missing(&server);
        expect_upload(
            &server,
            status_code(400)
                .append_header("Content-Type", "application/json")
                .body(r#"{"errors":["invalid rockspec"]}"#),
        );

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let err = upload(&project, &config, &package_db, None)
            .await
            .unwrap_err();
        match err {
            Error::Client(_, message) => assert_eq!(message, "invalid rockspec"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_outdated_tool_version() {
        let server = Server::run();
        expect_tool_version(&server, "0.0.1");

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let err = upload(&project, &config, &package_db, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::ToolCheck(check::tool::Error::ToolOutdated(..))
        ));
    }

    #[tokio::test]
    #[serial]
    async fn test_upload_unknown_user() {
        let server = Server::run();
        expect_tool_version(&server, TOOL_VERSION);
        expect_status(&server, 401);

        let (_project_dir, _config_dir, project, config, package_db) = ready(&server).await;
        let err = upload(&project, &config, &package_db, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::UserCheck(check::user::Error::UserNotFound)
        ));
    }
}
