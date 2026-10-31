use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::rockspec;

/// The wally client version advertised when downloading package contents.
/// The registry rejects requests without a sufficiently recent `Wally-Version` header.
pub(crate) const WALLY_VERSION: &str = "0.3.2";

pub mod name;
pub mod req;

pub(crate) use name::Name;
pub use req::Req;

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

/// The contents of a `wally.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Manifest {
    pub(crate) package: Package,

    #[serde(default)]
    pub(crate) place: PlaceInfo,

    #[serde(default)]
    pub(crate) dependencies: BTreeMap<String, req::Req>,

    #[serde(default)]
    pub(crate) server_dependencies: BTreeMap<String, req::Req>,

    #[serde(default)]
    pub(crate) dev_dependencies: BTreeMap<String, req::Req>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Package {
    /// The scope and name of the package, e.g. `jsdotperf/roact`.
    pub(crate) name: Name,

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

impl rockspec::display::AsLuaKV for req::Req {
    fn display_lua(&self) -> rockspec::display::LuaKV {
        rockspec::display::LuaKV {
            key: "wally".to_string(),
            value: rockspec::display::LuaValue::String(self.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    impl Manifest {
        fn parse(content: &str) -> Result<Self, toml::de::Error> {
            toml::from_str(content)
        }
    }

    impl Realm {
        fn is_dependency_valid(dep_type: Self, dep_realm: Self) -> bool {
            matches!(
                (dep_type, dep_realm),
                (Self::Server, _) | (Self::Shared, Self::Shared) | (Self::Dev, _)
            )
        }
    }

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
    fn realm_dependency_rules() {
        assert!(Realm::is_dependency_valid(Realm::Server, Realm::Shared));
        assert!(Realm::is_dependency_valid(Realm::Server, Realm::Server));
        assert!(Realm::is_dependency_valid(Realm::Shared, Realm::Shared));
        assert!(Realm::is_dependency_valid(Realm::Dev, Realm::Shared));
        assert!(!Realm::is_dependency_valid(Realm::Shared, Realm::Server));
        assert!(!Realm::is_dependency_valid(Realm::Shared, Realm::Dev));
    }
}
