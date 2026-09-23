#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashMap},
    fmt::{self, Display},
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

use git2::Repository;
use semver::{Version, VersionReq};
use serde::{de::Error as _, ser::Serializer, Deserialize, Deserializer, Serialize};
use thiserror::Error;
use url::Url;

use crate::lua_rockspec::{LuaModule, ModuleSpec, ParseLuaModuleError};

pub(crate) const MANIFEST_FILE_NAME: &str = "wally.toml";

/// The official wally package index.
pub(crate) const DEFAULT_INDEX_URL: &str = "https://github.com/UpliftGames/wally-index";

/// The wally client version advertised when downloading package contents.
/// The registry rejects requests without a sufficiently recent `Wally-Version` header.
pub(crate) const WALLY_VERSION: &str = "0.3.2";

/// A package name, of the form `scope/name`.
///
/// Both parts contain only lowercase letters, digits, and dashes (`-`), and
/// are at most 64 characters long.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PackageName {
    scope: PackageNamePart,
    name: PackageNamePart,
}

impl PackageName {
    pub(crate) fn scope(&self) -> &str {
        self.scope.as_str()
    }

    pub(crate) fn name(&self) -> &str {
        self.name.as_str()
    }
}

/// A part of a [`PackageName`]: lowercase letters, digits, and dashes, 1-64 chars.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PackageNamePart(String);

impl PackageNamePart {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PackageNamePart {
    type Err = PackageNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid || s.is_empty() || s.len() > 64 {
            return Err(PackageNameError::InvalidPart(s.to_string()));
        }
        Ok(Self(s.to_string()))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PackageNameError {
    #[error("wally package name part '{0}' is invalid: it must contain only lowercase letters, digits and '-', and be 1-64 characters long")]
    InvalidPart(String),
    #[error("wally package name must be of the form SCOPE/NAME")]
    InvalidFormat,
}

impl Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.scope.as_str(), self.name.as_str())
    }
}

impl FromStr for PackageName {
    type Err = PackageNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scope, name) = s.split_once('/').ok_or(PackageNameError::InvalidFormat)?;
        if name.contains('/') {
            return Err(PackageNameError::InvalidFormat);
        }
        Ok(Self {
            scope: scope.parse()?,
            name: name.parse()?,
        })
    }
}

impl Serialize for PackageName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// A requirement on a package: a name plus a SemVer range.
///
/// A bare version defaults to the `^` ("compatible") requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageReq {
    name: PackageName,
    version_req: VersionReq,
}

impl PackageReq {
    pub(crate) fn new(name: PackageName, version_req: VersionReq) -> Self {
        Self { name, version_req }
    }

    pub(crate) fn name(&self) -> &PackageName {
        &self.name
    }

    pub(crate) fn version_req(&self) -> &VersionReq {
        &self.version_req
    }

    pub(crate) fn matches(&self, name: &PackageName, version: &Version) -> bool {
        self.name == *name && self.version_req.matches(version)
    }
}

impl Display for PackageReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version_req)
    }
}

impl FromStr for PackageReq {
    type Err = PackageReqError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, req) = s.split_once('@').ok_or(PackageReqError::InvalidFormat)?;
        if req.is_empty() || req.chars().all(char::is_whitespace) {
            return Err(PackageReqError::InvalidFormat);
        }
        let name: PackageName = name.parse()?;
        let version_req = if Version::parse(req).is_ok() {
            VersionReq::parse(&format!("^{req}"))?
        } else {
            VersionReq::parse(req)?
        };
        Ok(Self::new(name, version_req))
    }
}

#[derive(Debug, Error)]
pub enum PackageReqError {
    #[error("wally package requirement must be of the form SCOPE/NAME@VERSION_REQ")]
    InvalidFormat,
    #[error(transparent)]
    Name(#[from] PackageNameError),
    #[error(transparent)]
    Version(#[from] semver::Error),
}

impl Serialize for PackageReq {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PackageReq {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// The realm a package can be used in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Realm {
    /// May depend on any realm.
    Server,
    /// May depend only on [`Realm::Shared`].
    Shared,
    /// Only valid as a root dependency.
    Dev,
}

impl Realm {
    pub(crate) fn is_dependency_valid(dep_type: Self, dep_realm: Self) -> bool {
        matches!(
            (dep_type, dep_realm),
            (Self::Server, _) | (Self::Shared, Self::Shared) | (Self::Dev, _)
        )
    }
}

/// The contents of a `wally.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Manifest {
    pub(crate) package: Package,

    #[serde(default)]
    pub(crate) place: PlaceInfo,

    #[serde(default)]
    pub(crate) dependencies: BTreeMap<String, PackageReq>,

    #[serde(default)]
    pub(crate) server_dependencies: BTreeMap<String, PackageReq>,

    #[serde(default)]
    pub(crate) dev_dependencies: BTreeMap<String, PackageReq>,
}

impl Manifest {
    pub(crate) fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join(MANIFEST_FILE_NAME);
        let content = std::fs::read_to_string(path).map_err(ManifestError::Io)?;
        Self::parse(&content)
    }

    pub(crate) fn parse(content: &str) -> Result<Self, ManifestError> {
        Ok(toml::from_str(content)?)
    }
}

#[derive(Debug, Error)]
pub(crate) enum ManifestError {
    #[error("failed to read {MANIFEST_FILE_NAME}")]
    Io(#[source] io::Error),
    #[error("failed to parse {MANIFEST_FILE_NAME}: {0}")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Package {
    /// The scope and name of the package, e.g. `jsdotperf/roact`.
    pub(crate) name: PackageName,

    /// The current SemVer version of the package.
    pub(crate) version: Version,

    /// The URL of the git index this package pulls its dependencies from.
    pub(crate) registry: Url,

    /// The realm (`shared`, `server`, or `dev`) this package can be used in.
    pub(crate) realm: Realm,

    /// A short description of the package.
    pub(crate) description: Option<String>,

    /// An SPDX license specifier for the package.
    pub(crate) license: Option<String>,

    /// The package's authors.
    #[serde(default)]
    pub(crate) authors: Vec<String>,

    /// Glob patterns of paths to include in the published package.
    #[serde(default)]
    pub(crate) include: Vec<String>,

    /// Glob patterns of paths to exclude from the published package.
    #[serde(default)]
    pub(crate) exclude: Vec<String>,

    /// Whether the package can be published.
    #[serde(default)]
    pub(crate) private: bool,

    /// The package homepage.
    pub(crate) homepage: Option<String>,

    /// The package source repository.
    pub(crate) repository: Option<String>,
}

/// Where shared and server packages are placed in the Roblox data model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct PlaceInfo {
    /// E.g. `game.ReplicatedStorage.Packages`.
    #[serde(default)]
    pub(crate) shared_packages: Option<String>,

    /// E.g. `game.ServerScriptService.Packages`.
    #[serde(default)]
    pub(crate) server_packages: Option<String>,
}

/// Configuration in a wally index's `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WallyIndexConfig {
    /// The HTTP registry that serves package contents.
    pub(crate) api: Url,

    #[serde(default)]
    pub(crate) fallback_registries: Vec<String>,
}

/// A local checkout of a wally package index.
#[derive(Debug, Clone)]
pub(crate) struct WallyIndex {
    path: PathBuf,
    config: WallyIndexConfig,
}

impl WallyIndex {
    pub(crate) fn new(path: PathBuf) -> Result<Self, WallyIndexError> {
        let config = serde_json::from_str(&std::fs::read_to_string(path.join("config.json"))?)?;
        Ok(Self { path, config })
    }

    /// Open a wally index, cloning it into `cache_dir` and updating it if needed.
    pub(crate) fn open(url: &Url, cache_dir: &Path) -> Result<Self, WallyIndexError> {
        let path = Self::cache_path(url, cache_dir);
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
        Self::new(path)
    }

    fn cache_path(url: &Url, cache_dir: &Path) -> PathBuf {
        let ident: String = url
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        cache_dir.join("wally").join("index").join(ident)
    }

    pub(crate) fn config(&self) -> &WallyIndexConfig {
        &self.config
    }

    /// All published versions of a package, newest first.
    pub(crate) fn versions(&self, name: &PackageName) -> Result<Vec<Manifest>, WallyIndexError> {
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

    /// The latest version of a package matching `req`.
    pub(crate) fn find(&self, req: &PackageReq) -> Result<Option<Manifest>, WallyIndexError> {
        Ok(self
            .versions(req.name())?
            .into_iter()
            .find(|manifest| req.matches(&manifest.package.name, &manifest.package.version)))
    }
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

/// Compute the module map for a wally package's contents.
pub(crate) fn modules_from_zip(
    bytes: &[u8],
) -> Result<HashMap<LuaModule, ModuleSpec>, WallyModulesError> {
    let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes))?;
    let mut modules = HashMap::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let path = Path::new(&name);
        if !is_lua_path(path) || is_test_file(path) {
            continue;
        }
        modules.insert(
            lua_module_from_entry(path)?,
            ModuleSpec::SourcePath(path.to_path_buf()),
        );
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
    use std::io::Write;

    const MANIFEST: &str = r#"
[package]
name = "jsdotperf/roact"
version = "1.4.2"
registry = "https://github.com/UpliftGames/wally-index"
realm = "shared"
description = "A declarative UI library"
license = "MIT"
authors = ["Johnny Morgan <johnny@test.com>"]
include = ["src", "*.lua"]
exclude = ["*.spec.lua"]
private = false
homepage = "https://github.com/jsdotperf/roact"
repository = "https://github.com/jsdotperf/roact"

[place]
shared-packages = "game.ReplicatedStorage.Packages"
server-packages = "game.ServerScriptService.Packages"

[dependencies]
promise = "evaera/promise@^3.1.0"
signal = "sleitnick/signal@2.0.0"

[server-dependencies]
data-store = "campfire/data-store@1.0.0"

[dev-dependencies]
testez = "roblox/testez@0.4.1"
"#;

    #[test]
    fn test_parse_manifest() {
        let manifest = Manifest::parse(MANIFEST).unwrap();
        assert_eq!(manifest.package.name.to_string(), "jsdotperf/roact");
        assert_eq!(manifest.package.version, Version::new(1, 4, 2));
        assert_eq!(manifest.package.realm, Realm::Shared);
        assert_eq!(manifest.package.license.as_deref(), Some("MIT"));
        assert_eq!(
            manifest.place.shared_packages.as_deref(),
            Some("game.ReplicatedStorage.Packages")
        );
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(manifest.server_dependencies.len(), 1);
        assert_eq!(manifest.dev_dependencies.len(), 1);
    }

    #[test]
    fn bare_version_defaults_to_caret() {
        let req: PackageReq = "sleitnick/signal@2.0.0".parse().unwrap();
        assert_eq!(req.version_req(), &VersionReq::parse("^2.0.0").unwrap());
        let name: PackageName = "sleitnick/signal".parse().unwrap();
        assert!(req.matches(&name, &Version::new(2, 0, 0)));
        assert!(!req.matches(&name, &Version::new(3, 0, 0)));
    }

    #[test]
    fn realm_dependency_rules() {
        assert!(Realm::is_dependency_valid(Realm::Server, Realm::Shared));
        assert!(Realm::is_dependency_valid(Realm::Server, Realm::Server));
        assert!(Realm::is_dependency_valid(Realm::Shared, Realm::Shared));
        assert!(Realm::is_dependency_valid(Realm::Dev, Realm::Shared));
        assert!(!Realm::is_dependency_valid(Realm::Shared, Realm::Server));
        assert!(!Realm::is_dependency_valid(Realm::Shared, Realm::Dev));
    }

    #[test]
    fn test_parse_package_name() {
        assert!("Upper-Skewer".parse::<PackageNamePart>().is_err());
        assert!("snake_case".parse::<PackageNamePart>().is_err());
        assert!("hello/world/foo".parse::<PackageName>().is_err());
        assert!("hello/world".parse::<PackageName>().is_ok());
    }

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

        let req: PackageReq = "evaera/promise@2".parse().unwrap();
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
        let req: PackageReq = "evaera/promise@1".parse().unwrap();
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
}
