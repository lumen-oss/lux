use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

use bytes::Bytes;
use git2::Repository;
use miette::Diagnostic;
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{
    config::Config,
    lockfile::{OptState, PinnedState, RemotePackageSourceUrl},
    lua_rockspec::{LuaModule, ModuleSpec, ParseLuaModuleError, RemoteLuaRockspec, RockSourceSpec},
    package::{
        wally::{self, Manifest, PackageName as WallyPackageName, PackageReq as WallyPackageReq},
        PackageName, PackageReq, PackageSpec, PackageVersion, PackageVersionParseError,
        PackageVersionReq, RemotePackage,
    },
    remote_package_source::RemotePackageSource,
    reqwest::RequestError,
    rockspec::lua_dependency::LuaDependencySpec,
};

use super::FoundPackage;

const DEFAULT_INDEX_URL: &str = "https://github.com/UpliftGames/wally-index";

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum WallyDiscoverError {
    #[error("failed to resolve wally package")]
    #[diagnostic(help(
        "check your network connection and that the wally registry URL is correct."
    ))]
    Index(#[from] WallyIndexError),
    #[error("failed to inspect wally package contents")]
    #[diagnostic(help(
        "the registry may have returned a corrupt package archive; try again, or report it to the package author."
    ))]
    Modules(#[from] WallyModulesError),
    #[error("failed to parse wally package version")]
    #[diagnostic(help(
        "the package manifest contains an invalid version; report it to the package author."
    ))]
    Version(#[from] PackageVersionParseError),
    #[error("wally package '{0}' was not found in any registry")]
    #[diagnostic(help(
        "check the package name and version, or add its registry to `extra_wally_registries` in your Lux config."
    ))]
    NotFound(WallyPackageReq),
    #[error("failed to construct the wally package content URL '{url}'")]
    #[diagnostic(help("the registry's `config.json` may contain an invalid `api` URL."))]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("wally package '{0}' is missing its content URL in the lockfile")]
    #[diagnostic(help("the lockfile may be outdated; regenerate it."))]
    MissingSourceUrl(String),
    #[error("failed to determine the wally package name from '{0}'")]
    #[diagnostic(help(
        "the lockfile may contain an invalid wally source; run `lx update` to regenerate it."
    ))]
    InvalidSourceUrl(String),
    #[error("failed to parse the wally package requirement")]
    #[diagnostic(help("the lockfile may contain an invalid wally version; regenerate it."))]
    Requirement(#[from] semver::Error),
    #[error("client error")]
    #[diagnostic(transparent)]
    Request(#[from] RequestError),
}

impl From<reqwest::Error> for WallyDiscoverError {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

/// Discover a wally package from the configured registries.
pub(crate) async fn discover(
    package_req: &PackageReq,
    wally_req: &WallyPackageReq,
    config: &Config,
) -> Result<FoundPackage, WallyDiscoverError> {
    let mut registries = config.extra_wally_registries().clone();
    registries.push(unsafe { Url::parse(DEFAULT_INDEX_URL).unwrap_unchecked() });
    for registry in &registries {
        let index = WallyIndex::open(registry, config.cache_dir())?;
        let Some(manifest) = index.find(wally_req)? else {
            continue;
        };
        let content_url = content_url(&index, wally_req, &manifest)?;
        let contents = fetch_contents(&content_url, config).await?;
        return found_package(package_req, manifest, content_url, &contents, registry);
    }
    Err(WallyDiscoverError::NotFound(wally_req.clone()))
}

/// Discover a wally package that is pinned in a lockfile.
pub(crate) async fn discover_locked(
    remote: &RemotePackage,
    index_url: &Url,
    config: &Config,
) -> Result<FoundPackage, WallyDiscoverError> {
    let package_req = remote.spec.clone().into_package_req();
    let wally_req = wally_req_from_locked(remote)?;
    let index = WallyIndex::open(index_url, config.cache_dir())?;
    let manifest = index
        .find(&wally_req)?
        .ok_or_else(|| WallyDiscoverError::NotFound(wally_req.clone()))?;
    let content_url = content_url(&index, &wally_req, &manifest)?;
    let contents = fetch_contents(&content_url, config).await?;
    found_package(&package_req, manifest, content_url, &contents, index_url)
}

fn wally_req_from_locked(remote: &RemotePackage) -> Result<WallyPackageReq, WallyDiscoverError> {
    let content_url = if let Some(RemotePackageSourceUrl::Url { url }) = &remote.source_url {
        url
    } else {
        return Err(WallyDiscoverError::MissingSourceUrl(
            remote.spec.name().to_string(),
        ));
    };
    let mut segments = content_url.path_segments().into_iter().flatten().rev();
    let _version = segments.next();
    let name = segments.next();
    let scope = segments.next();
    let (Some(scope), Some(name)) = (scope, name) else {
        return Err(WallyDiscoverError::InvalidSourceUrl(
            content_url.to_string(),
        ));
    };
    let name: WallyPackageName = format!("{scope}/{name}")
        .parse()
        .map_err(|_| WallyDiscoverError::InvalidSourceUrl(content_url.to_string()))?;
    let version_req =
        VersionReq::parse(&format!("={}", remote.spec.version().to_version_string()))?;
    Ok(WallyPackageReq::new(name, version_req))
}

fn content_url(
    index: &WallyIndex,
    wally_req: &WallyPackageReq,
    manifest: &Manifest,
) -> Result<Url, WallyDiscoverError> {
    index
        .config()
        .api
        .join(&format!(
            "/v1/package-contents/{}/{}/{}",
            wally_req.name().scope(),
            wally_req.name().name(),
            manifest.package.version
        ))
        .map_err(|source| WallyDiscoverError::Url {
            url: index.config().api.to_string(),
            source,
        })
}

fn dependencies(manifest: Manifest) -> Vec<LuaDependencySpec> {
    manifest
        .dependencies
        .into_iter()
        .map(|(alias, dep_req)| LuaDependencySpec {
            package_req: PackageReq {
                name: PackageName::new(alias),
                version_req: PackageVersionReq::SemVer(dep_req.version_req().clone()),
            },
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: Some(RockSourceSpec::Wally(dep_req)),
        })
        .collect()
}

fn found_package(
    package_req: &PackageReq,
    manifest: Manifest,
    content_url: Url,
    contents: &[u8],
    registry: &Url,
) -> Result<FoundPackage, WallyDiscoverError> {
    let version = PackageVersion::parse(&manifest.package.version.to_string())?;
    let package_spec = PackageSpec::new(package_req.name().clone(), version);
    let modules = modules_from_zip(contents)?;
    let source_spec = RockSourceSpec::Url(content_url.clone());
    let rockspec = RemoteLuaRockspec::from_source_spec(
        package_spec.clone(),
        source_spec,
        dependencies(manifest),
        modules,
    );
    let remote = RemotePackage::new(
        package_spec,
        RemotePackageSource::Wally(registry.clone()),
        Some(RemotePackageSourceUrl::Url { url: content_url }),
    );
    Ok(FoundPackage {
        remote,
        rockspec,
        artifact: None,
    })
}

async fn fetch_contents(url: &Url, config: &Config) -> Result<Bytes, WallyDiscoverError> {
    let request = crate::reqwest::http_client(config)?.get(url.clone());
    let request = wally::apply_content_headers(request, url);
    let response = request.send().await?.error_for_status()?.bytes().await?;
    Ok(response)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WallyIndexConfig {
    api: Url,

    #[allow(dead_code)]
    #[serde(default)]
    fallback_registries: Vec<String>,
}

#[derive(Debug, Clone)]
struct WallyIndex {
    path: PathBuf,
    config: WallyIndexConfig,
}

impl WallyIndex {
    fn new(path: PathBuf) -> Result<Self, WallyIndexError> {
        let config = serde_json::from_str(&std::fs::read_to_string(path.join("config.json"))?)?;
        Ok(Self { path, config })
    }

    fn open(url: &Url, cache_dir: &Path) -> Result<Self, WallyIndexError> {
        Self::new(checkout(url, cache_dir)?)
    }

    fn config(&self) -> &WallyIndexConfig {
        &self.config
    }

    fn versions(&self, name: &WallyPackageName) -> Result<Vec<Manifest>, WallyIndexError> {
        let path = self.path.join(name.scope()).join(name.name());
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut versions: Vec<Manifest> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(WallyIndexError::from))
            .collect::<Result<_, _>>()?;
        versions.sort_by(|a, b| b.package.version.cmp(&a.package.version));
        Ok(versions)
    }

    fn find(&self, req: &WallyPackageReq) -> Result<Option<Manifest>, WallyIndexError> {
        Ok(self
            .versions(req.name())?
            .into_iter()
            .find(|manifest| req.matches(&manifest.package.name, &manifest.package.version)))
    }
}

fn checkout(url: &Url, cache_dir: &Path) -> Result<PathBuf, WallyIndexError> {
    let path = cache_path(url, cache_dir);
    if path.join(".git").is_dir() {
        let repo = Repository::open(&path)?;
        let mut remote = repo.find_remote("origin")?;
        remote.fetch(&[] as &[&str], None, None)?;
        let head = repo.refname_to_id("refs/remotes/origin/HEAD")?;
        repo.reset(&repo.find_object(head, None)?, git2::ResetType::Hard, None)?;
    } else {
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Repository::clone(url.as_str(), &path)?;
    }
    Ok(path)
}

fn cache_path(url: &Url, cache_dir: &Path) -> PathBuf {
    let ident: String = url
        .as_str()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    cache_dir.join("wally").join("index").join(ident)
}

#[derive(Debug, Error)]
pub enum WallyIndexError {
    #[error("failed to read wally index")]
    Io(#[from] io::Error),
    #[error("failed to parse wally index")]
    Json(#[from] serde_json::Error),
    #[error("failed to fetch wally index")]
    Git(#[from] git2::Error),
}

fn modules_from_zip(bytes: &[u8]) -> Result<HashMap<LuaModule, ModuleSpec>, WallyModulesError> {
    let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes))?;
    let mut modules = HashMap::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.is_dir() {
            continue;
        }
        match file.enclosed_name() {
            Some(path) if is_lua_path(&path) && !is_test_file(&path) => {
                modules.insert(lua_module_from_entry(&path)?, ModuleSpec::SourcePath(path));
            }
            _ => {}
        }
    }
    Ok(modules)
}

fn is_lua_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("lua" | "luau")
    )
}

fn is_test_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with(".test") || stem.ends_with(".spec"))
}

fn lua_module_from_entry(path: &Path) -> Result<LuaModule, ParseLuaModuleError> {
    let stripped: PathBuf = match path.components().next() {
        Some(component)
            if matches!(component.as_os_str().to_str(), Some("src" | "lua" | "lib")) =>
        {
            path.components().skip(1).collect()
        }
        _ => path.to_path_buf(),
    };
    if stripped
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        let mut file = stripped;
        file.set_extension("");
        LuaModule::from_pathbuf(file)
    } else {
        let mut module = LuaModule::from_pathbuf(stripped.to_path_buf())?;
        if matches!(
            stripped.file_name().and_then(|name| name.to_str()),
            Some("init.lua" | "init.luau")
        ) {
            module = module.join(unsafe { &LuaModule::from_str("init").unwrap_unchecked() });
        }
        Ok(module)
    }
}

#[derive(Debug, Error)]
pub enum WallyModulesError {
    #[error("failed to read package contents archive")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid module path in package contents")]
    Module(#[from] ParseLuaModuleError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, io::Write};

    use httptest::{matchers::request, responders::status_code, Expectation, Server};
    use semver::Version;

    use crate::{
        config::ConfigBuilder,
        package::wally::{Package, PlaceInfo, Realm},
    };

    fn package(version: Version) -> Manifest {
        Manifest {
            package: Package {
                name: "evaera/promise".parse().unwrap(),
                version,
                registry: Url::parse("https://github.com/UpliftGames/wally-index").unwrap(),
                realm: Realm::Shared,
                description: None,
                license: None,
                authors: Vec::new(),
                include: Vec::new(),
                exclude: Vec::new(),
                private: false,
                homepage: None,
                repository: None,
            },
            place: PlaceInfo::default(),
            dependencies: BTreeMap::new(),
            server_dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
        }
    }

    fn local_index(api: &Url) -> (assert_fs::TempDir, Url) {
        let dir = assert_fs::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            format!(r#"{{"api":"{api}"}}"#),
        )
        .unwrap();
        let pkg_path = dir.path().join("evaera").join("promise");
        std::fs::create_dir_all(pkg_path.parent().unwrap()).unwrap();
        let manifest = serde_json::to_string(&package(Version::new(1, 0, 0))).unwrap();
        std::fs::write(pkg_path, format!("{manifest}\n")).unwrap();

        let repo = git2::Repository::init(dir.path()).unwrap();
        {
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("test", "test@example.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        let url = Url::from_directory_path(dir.path()).unwrap();
        (dir, url)
    }

    fn wally_package_content_zip() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let options = zip::write::SimpleFileOptions::default();
            for entry in ["init.luau", "src/foo.luau"] {
                zip.start_file(entry, options).unwrap();
                zip.write_all(b"return {}").unwrap();
            }
            zip.finish().unwrap();
        }
        bytes
    }

    #[test]
    fn resolves_from_index() {
        let dir = assert_fs::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"api":"https://api.wally.run"}"#,
        )
        .unwrap();
        let pkg_path = dir.path().join("evaera").join("promise");
        std::fs::create_dir_all(pkg_path.parent().unwrap()).unwrap();
        let jsonl = [Version::new(2, 4, 0), Version::new(3, 1, 0)]
            .into_iter()
            .map(|version| serde_json::to_string(&package(version)).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(pkg_path, format!("{jsonl}\n")).unwrap();

        let index = WallyIndex::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(
            index.config().api,
            Url::parse("https://api.wally.run").unwrap()
        );

        let req: WallyPackageReq = "evaera/promise@2".parse().unwrap();
        assert_eq!(
            index.find(&req).unwrap().unwrap().package.version,
            Version::new(2, 4, 0)
        );
    }

    #[test]
    fn opens_local_index() {
        let remote = assert_fs::TempDir::new().unwrap();
        std::fs::write(
            remote.path().join("config.json"),
            r#"{"api":"https://api.wally.run"}"#,
        )
        .unwrap();
        let pkg_path = remote.path().join("evaera").join("promise");
        std::fs::create_dir_all(pkg_path.parent().unwrap()).unwrap();
        let jsonl = serde_json::to_string(&package(Version::new(1, 0, 0))).unwrap();
        std::fs::write(&pkg_path, format!("{jsonl}\n")).unwrap();

        let repo = git2::Repository::init(remote.path()).unwrap();
        {
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("test", "test@example.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        let cache = assert_fs::TempDir::new().unwrap();
        let url = Url::from_directory_path(remote.path()).unwrap();
        let index = WallyIndex::open(&url, cache.path()).unwrap();
        let req: WallyPackageReq = "evaera/promise@1".parse().unwrap();
        assert_eq!(
            index.find(&req).unwrap().unwrap().package.version,
            Version::new(1, 0, 0)
        );
    }

    #[test]
    fn computes_modules_from_zip() {
        let mut bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let options = zip::write::SimpleFileOptions::default();
            for entry in [
                "init.luau",
                "init.test.luau",
                "src/foo.luau",
                "src/bar/baz.luau",
                "sub/init.lua",
                "foo.spec.lua",
                "wally.toml",
            ] {
                zip.start_file(entry, options).unwrap();
                zip.write_all(b"").unwrap();
            }
            zip.finish().unwrap();
        }

        let modules = modules_from_zip(&bytes).unwrap();
        let mut names: Vec<_> = modules.keys().map(|module| module.to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["bar.baz", "foo", "init", "sub.init"]);
        assert_eq!(
            modules.get(&LuaModule::from_str("foo").unwrap()),
            Some(&ModuleSpec::SourcePath("src/foo.luau".into()))
        );
    }

    #[test]
    fn derives_wally_requirement_from_locked_package() {
        let remote = RemotePackage::new(
            PackageSpec::parse("promise".into(), "1.0.0".into()).unwrap(),
            RemotePackageSource::Wally(Url::parse("file:///tmp/wally-index").unwrap()),
            Some(RemotePackageSourceUrl::Url {
                url: Url::parse("https://api.wally.run/v1/package-contents/evaera/promise/1.0.0")
                    .unwrap(),
            }),
        );

        let req = wally_req_from_locked(&remote).unwrap();
        assert_eq!(req.name().to_string(), "evaera/promise");
        assert_eq!(req.version_req(), &VersionReq::parse("=1.0.0").unwrap());
    }

    #[test]
    fn rejects_locked_package_without_content_url() {
        let remote = RemotePackage::new(
            PackageSpec::parse("promise".into(), "1.0.0".into()).unwrap(),
            RemotePackageSource::Wally(Url::parse("file:///tmp/wally-index").unwrap()),
            None,
        );

        assert!(matches!(
            wally_req_from_locked(&remote),
            Err(WallyDiscoverError::MissingSourceUrl(_))
        ));
    }

    #[tokio::test]
    async fn resolves_locked_package_from_registry() {
        let server = Server::run();
        let api_url = Url::parse(&server.url("/").to_string()).unwrap();
        let (_index_dir, index_url) = local_index(&api_url);

        let content_path = "/v1/package-contents/evaera/promise/1.0.0";
        server.expect(
            Expectation::matching(request::method_path("GET", content_path))
                .respond_with(status_code(200).body(wally_package_content_zip())),
        );

        let content_url = Url::parse(&server.url(content_path).to_string()).unwrap();
        let remote = RemotePackage::new(
            PackageSpec::parse("promise".into(), "1.0.0".into()).unwrap(),
            RemotePackageSource::Wally(index_url.clone()),
            Some(RemotePackageSourceUrl::Url { url: content_url }),
        );
        let cache = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache.path().to_path_buf()))
            .build()
            .unwrap();

        let found = discover_locked(&remote, &index_url, &config).await.unwrap();
        assert_eq!(found.remote.spec.name().to_string(), "promise");
        assert_eq!(found.remote.spec.version().to_version_string(), "1.0.0");
        assert_eq!(found.remote.source, RemotePackageSource::Wally(index_url));
        assert!(found.artifact.is_none());
    }

    #[tokio::test]
    async fn discovers_from_extra_registry() {
        let server = Server::run();
        let api_url = Url::parse(&server.url("/").to_string()).unwrap();
        let (_index_dir, index_url) = local_index(&api_url);

        let content_path = "/v1/package-contents/evaera/promise/1.0.0";
        server.expect(
            Expectation::matching(request::method_path("GET", content_path))
                .respond_with(status_code(200).body(wally_package_content_zip())),
        );

        let cache = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache.path().to_path_buf()))
            .extra_wally_registries(Some(vec![index_url.clone()]))
            .build()
            .unwrap();
        let package_req = PackageReq::new("promise".into(), Some("1.0.0".into())).unwrap();
        let wally_req: WallyPackageReq = "evaera/promise@1".parse().unwrap();

        let found = discover(&package_req, &wally_req, &config).await.unwrap();
        assert_eq!(found.remote.spec.name().to_string(), "promise");
        assert_eq!(found.remote.source, RemotePackageSource::Wally(index_url));
    }
}
