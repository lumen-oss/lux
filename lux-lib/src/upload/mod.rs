use std::{env, io};

use crate::operations::SearchAndDownloadError;
use crate::package::SpecRevIterator;
use crate::package_db::PackageDB;
use crate::project::project_toml::{ProjectTomlError, RemoteProjectTomlValidationError};
use crate::rockspec::Rockspec;
use crate::TOOL_VERSION;
use crate::{config::Config, project::Project};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_enum_str::Serialize_enum_str;
use thiserror::Error;
use url::Url;

#[cfg(feature = "gpgme")]
use gpgme::{Context, Data};
#[cfg(feature = "gpgme")]
use std::io::Read;

const TFA_TOKEN_HEADER: &str = "X-TFA-Token";

/// A rocks package uploader, providing fine-grained control
/// over how a package should be uploaded.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct ProjectUpload<'a> {
    project: &'a Project,
    api_key: Option<ApiKey>,
    tfa_code: Option<String>,
    #[cfg(feature = "gpgme")]
    sign_protocol: SignatureProtocol,
    config: &'a Config,
    package_db: &'a PackageDB,
}

impl<State> ProjectUploadBuilder<'_, State>
where
    State: project_upload_builder::State + project_upload_builder::IsComplete,
{
    /// Upload a package to a luarocks server.
    pub async fn upload_to_luarocks(self) -> Result<(), UploadError> {
        let args = self._build();
        upload_from_project(args).await
    }
}

#[derive(Deserialize, Debug)]
pub struct VersionCheckResponse {
    version: String,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum ToolCheckError {
    #[error("error parsing tool check URL")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("error sending HTTP request")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] reqwest::Error),
    #[error(r#"Lux is out of date with {0}'s expected tool version.
    Lux is at version {TOOL_VERSION}, server is at {server_version}"#, server_version = _1.version)]
    #[diagnostic(help("ensure you have an up-to-date version of Lux installed"))]
    ToolOutdated(String, VersionCheckResponse),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum UserCheckError {
    #[error("error parsing user check URL")]
    #[diagnostic(help("check your server configuration"))]
    Parse(#[from] url::ParseError),
    #[error(transparent)]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] reqwest::Error),
    #[diagnostic(help(
        "ensure that the API key provided via the $LUX_API_KEY environemnt variable is correct"
    ))]
    #[error("invalid API key provided")]
    UserNotFound,
    #[error("server '{0}' responded with error status: '{1}'")]
    #[diagnostic(help("the server may be temporarily unavailable"))]
    Server(Url, StatusCode),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum RockCheckError {
    #[error("parse error while checking rock status on server")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("HTTP request error while checking rock status on server")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] reqwest::Error),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(transparent)]
pub enum UploadError {
    #[error("error parsing upload URL")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("HTPP request error while uploading")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] reqwest::Error),
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
    RockCheck(#[from] RockCheckError),
    #[error("a package with the same rockspec content already exists on the server: '{0}'")]
    #[diagnostic(help("try uploading a rockspec with a different version"))]
    RockExists(Url),
    #[error("unable to read rockspec")]
    #[diagnostic(help("check that the rockspec exists and is readable"))]
    RockspecRead(#[from] std::io::Error),
    #[cfg(feature = "gpgme")]
    #[error("GPG signing failed")]
    #[diagnostic(help(
        r#"ensure that a GPG agent is running and that a valid GPG signing key is registered.
          if you'd like to skip the signing step, supply `--sign-protocol=none`
        "#
    ))]
    Signature(#[from] gpgme::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ToolCheck(#[from] ToolCheckError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    UserCheck(#[from] UserCheckError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ApiKeyUnspecified(#[from] ApiKeyUnspecified),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ValidationError(#[from] RemoteProjectTomlValidationError),
    #[error("unsupported version: '{0}'")]
    #[diagnostic(help("Lux can upload packages with a SemVer version, 'dev' or 'scm'"))]
    UnsupportedVersion(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Rockspec(#[from] ProjectTomlError),
    #[error("the maximum supported number of rockspec revisions per version has been exceeded")]
    #[diagnostic(help("bump the version to a version that has not yet been published"))]
    MaxSpecRevsExceeded,
    #[error("rock already exists on server. Error downloading existing rockspec")]
    #[diagnostic(forward(0))]
    SearchAndDownload(#[from] Box<SearchAndDownloadError>),
    #[error("error computing rockspec hash")]
    Hash(io::Error),
    #[error("the 2FA code '{0}' was rejected by the server: {1}")]
    #[diagnostic(help("it may have expired; try again with a new code."))]
    TfaCodeRejected(String, String),
}

impl From<SearchAndDownloadError> for UploadError {
    fn from(source: SearchAndDownloadError) -> Self {
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

#[derive(Serialize_enum_str, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
#[cfg(not(feature = "gpgme"))]
pub enum SignatureProtocol {
    #[default]
    None,
}

#[derive(Serialize_enum_str, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
#[cfg(feature = "gpgme")]
pub enum SignatureProtocol {
    None,
    Assuan,
    CMS,
    #[default]
    Default,
    G13,
    GPGConf,
    OpenPGP,
    Spawn,
    UIServer,
}

#[cfg(feature = "gpgme")]
impl From<SignatureProtocol> for gpgme::Protocol {
    fn from(val: SignatureProtocol) -> Self {
        match val {
            SignatureProtocol::Default => gpgme::Protocol::Default,
            SignatureProtocol::OpenPGP => gpgme::Protocol::OpenPgp,
            SignatureProtocol::CMS => gpgme::Protocol::Cms,
            SignatureProtocol::GPGConf => gpgme::Protocol::GpgConf,
            SignatureProtocol::Assuan => gpgme::Protocol::Assuan,
            SignatureProtocol::G13 => gpgme::Protocol::G13,
            SignatureProtocol::UIServer => gpgme::Protocol::UiServer,
            SignatureProtocol::Spawn => gpgme::Protocol::Spawn,
            SignatureProtocol::None => unreachable!(),
        }
    }
}

#[tracing::instrument(
    name = "Uploading",
    level = "info",
    skip_all,
    fields(project = args.project.toml().package().to_string()),
)]
async fn upload_from_project(args: ProjectUpload<'_>) -> Result<(), UploadError> {
    let project = args.project;
    let api_key = args.api_key.unwrap_or(ApiKey::new()?);
    #[cfg(feature = "gpgme")]
    let protocol = args.sign_protocol;
    let config = args.config;
    let package_db = args.package_db;

    let client = crate::reqwest::https_client(args.config)?;

    helpers::ensure_tool_version(client, config.server()).await?;
    helpers::ensure_user_exists(client, &api_key, config.server()).await?;

    let (rockspec, rockspec_content) =
        helpers::generate_rockspec(project, client, &api_key, config, package_db).await?;

    #[cfg(not(feature = "gpgme"))]
    let signed: Option<String> = None;

    #[cfg(feature = "gpgme")]
    let signed = if let SignatureProtocol::None = protocol {
        None
    } else {
        let mut ctx = Context::from_protocol(protocol.into())?;
        let mut signature = Data::new()?;

        ctx.set_armor(true);
        ctx.sign_detached(rockspec_content.clone(), &mut signature)?;

        let mut signature_str = String::new();
        signature.read_to_string(&mut signature_str)?;

        Some(signature_str)
    };

    let rockspec = Part::text(rockspec_content)
        .file_name(format!(
            "{}-{}.rockspec",
            rockspec.package(),
            rockspec.version()
        ))
        .mime_str("application/octet-stream")?;

    let multipart = {
        let multipart = Form::new().part("rockspec_file", rockspec);

        match signed {
            Some(signature) => {
                let part = Part::text(signature).file_name("project.rockspec.sig");
                multipart.part("rockspec_sig", part)
            }
            None => multipart,
        }
    };

    let mut request = client
        .post(unsafe { helpers::url_for_method(config.server(), &api_key, "upload")? })
        .multipart(multipart);

    if let Some(code) = args.tfa_code {
        let token = helpers::verify_tfa_code(client, config.server(), &api_key, &code).await?;
        request = request.header(TFA_TOKEN_HEADER, unsafe { token.get() });
    }

    let response = request.send().await?;

    let status = response.status();
    if status.is_server_error() {
        Err(UploadError::Server(config.server().clone(), status))
    } else if status.is_success() {
        Ok(())
    } else {
        let response = response.json::<LuarocksErrorResponse>().await?;
        let errors = response.errors.into_iter().join("\n");
        Err(UploadError::Client(config.server().clone(), errors))
    }
}

mod helpers {
    use std::collections::HashMap;

    use super::*;
    use crate::hash::HasIntegrity;
    use crate::operations::Download;
    use crate::package::{PackageName, PackageSpec, PackageVersion};
    use crate::project::project_toml::RemoteProjectToml;
    use crate::upload::RockCheckError;
    use crate::upload::{ToolCheckError, UserCheckError};
    use itertools::Itertools;
    use reqwest::Client;
    use ssri::Integrity;
    use url::Url;

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

    #[tracing::instrument(level = "trace", skip(client))]
    pub(crate) async fn ensure_tool_version(
        client: &Client,
        server_url: &Url,
    ) -> Result<(), ToolCheckError> {
        let url = server_url.join("api/tool_version")?;
        let response: VersionCheckResponse = client
            .post(url)
            .json(&("current", TOOL_VERSION))
            .send()
            .await?
            .json()
            .await?;

        if response.version == TOOL_VERSION {
            Ok(())
        } else {
            Err(ToolCheckError::ToolOutdated(
                server_url.to_string(),
                response,
            ))
        }
    }

    #[tracing::instrument(level = "trace", skip(client, api_key))]
    pub(crate) async fn verify_tfa_code(
        client: &Client,
        server_url: &Url,
        api_key: &ApiKey,
        tfa_code: &str,
    ) -> Result<TfaToken, UploadError> {
        let response = client
            .get(unsafe { url_for_method(server_url, api_key, "verify_tfa")? })
            .query(&[("code", tfa_code)])
            .send()
            .await?;
        let status = response.status();
        if status.is_server_error() {
            Err(UploadError::Server(server_url.clone(), status))
        } else {
            match response.json::<LuarocksTfaVerificationResponse>().await? {
                LuarocksTfaVerificationResponse::Success(LuarocksTfaVerificationSuccess {
                    tfa_token,
                }) => Ok(tfa_token),
                LuarocksTfaVerificationResponse::Failure(LuarocksErrorResponse { errors }) => {
                    Err(UploadError::TfaCodeRejected(
                        tfa_code.to_string(),
                        errors.into_iter().join("\n"),
                    ))
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(client, api_key))]
    pub(crate) async fn ensure_user_exists(
        client: &Client,
        api_key: &ApiKey,
        server_url: &Url,
    ) -> Result<(), UserCheckError> {
        let response = client
            .get(unsafe { url_for_method(server_url, api_key, "status")? })
            .send()
            .await?;
        let status = response.status();
        if status.is_client_error() {
            Err(UserCheckError::UserNotFound)
        } else if status.is_server_error() {
            Err(UserCheckError::Server(server_url.clone(), status))
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) async fn generate_rockspec(
        project: &Project,
        client: &Client,
        api_key: &ApiKey,
        config: &Config,
        package_db: &PackageDB,
    ) -> Result<(RemoteProjectToml, String), UploadError> {
        for specrev in SpecRevIterator::new() {
            let rockspec = project.toml().into_remote(Some(specrev))?;

            let rockspec_content = rockspec.to_lua_remote_rockspec_string()?;

            if let PackageVersion::StringVer(ver) = rockspec.version() {
                return Err(UploadError::UnsupportedVersion(ver.to_string()));
            }
            if helpers::rock_exists(
                client,
                api_key,
                rockspec.package(),
                rockspec.version(),
                config.server(),
            )
            .await?
            {
                let package =
                    PackageSpec::new(rockspec.package().clone(), rockspec.version().clone());
                let existing_rockspec = Download::new(&package.into(), config)
                    .package_db(package_db)
                    .download_rockspec()
                    .await?
                    .rockspec;
                let existing_rockspec_hash =
                    existing_rockspec.hash().await.map_err(UploadError::Hash)?;
                let rockspec_content_hash = Integrity::from(&rockspec_content);
                if existing_rockspec_hash
                    .matches(&rockspec_content_hash)
                    .is_some()
                {
                    return Err(UploadError::RockExists(config.server().clone()));
                }
            } else {
                return Ok((rockspec, rockspec_content));
            }
        }
        Err(UploadError::MaxSpecRevsExceeded)
    }

    #[tracing::instrument(level = "trace", skip(client, api_key))]
    async fn rock_exists(
        client: &Client,
        api_key: &ApiKey,
        name: &PackageName,
        version: &PackageVersion,
        server: &Url,
    ) -> Result<bool, RockCheckError> {
        let server_response_raw_json = client
            .get(unsafe { url_for_method(server, api_key, "check_rockspec")? })
            .query(&(
                ("package", name.to_string()),
                ("version", version.to_string()),
            ))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let response_map: Option<HashMap<String, serde_json::Value>> =
            serde_json::from_str(&server_response_raw_json).ok();
        Ok(response_map.is_some_and(|response_map| {
            response_map.contains_key("module") && response_map.contains_key("version")
        }))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::config::ConfigBuilder;
    use crate::lua_version::LuaVersion;
    use assert_fs::prelude::*;
    use httptest::matchers::{contains, request};
    use httptest::responders::{json_encoded, status_code, Responder};
    use httptest::{Expectation, Server};
    use serial_test::serial;

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
        let config = ConfigBuilder::default()
            .server(Some(Url::parse(&server.url_str("/")).unwrap()))
            .data_dir(Some(config_dir.path().to_path_buf()))
            .cache_dir(Some(config_dir.path().to_path_buf()))
            .no_manifest_cache(Some(true))
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let package_db = PackageDB::from_config(&config).await.unwrap();

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

    async fn upload(
        project: &Project,
        config: &Config,
        package_db: &PackageDB,
        tfa_code: Option<String>,
    ) -> Result<(), UploadError> {
        let upload = ProjectUpload::new()
            .project(project)
            .config(config)
            .api_key(ApiKey::from(API_KEY))
            .maybe_tfa_code(tfa_code)
            .package_db(package_db);
        #[cfg(feature = "gpgme")]
        let upload = upload.sign_protocol(SignatureProtocol::None);
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
        assert!(matches!(err, UploadError::TfaCodeRejected(..)));
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
            UploadError::Server(_, StatusCode::INTERNAL_SERVER_ERROR)
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
            UploadError::Client(_, message) => assert_eq!(message, "invalid rockspec"),
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
            UploadError::ToolCheck(ToolCheckError::ToolOutdated(..))
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
            UploadError::UserCheck(UserCheckError::UserNotFound)
        ));
    }
}
