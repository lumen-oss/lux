use std::fmt::Display;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::rockspec;
use crate::rockspec::lua_dependency;

pub mod db;
mod outdated;
pub mod req;
pub mod spec;
pub mod version;

pub use outdated::*;
pub use req::Req;
pub use spec::Spec;
pub use version::{Version, VersionReq};

pub(crate) use version::SpecRevIterator;

/// Wrapper structs for proper serialization of various dependency types.
pub(crate) struct Dependencies<'a>(pub(crate) &'a Vec<lua_dependency::Spec>);

pub(crate) struct BuildDependencies<'a>(pub(crate) &'a Vec<lua_dependency::Spec>);

impl rockspec::display::AsLuaKV for Dependencies<'_> {
    fn display_lua(&self) -> rockspec::display::LuaKV {
        rockspec::display::LuaKV {
            key: "dependencies".to_string(),
            value: rockspec::display::LuaValue::List(
                self.0
                    .iter()
                    .map(|req| rockspec::display::LuaValue::String(req.to_string()))
                    .collect(),
            ),
        }
    }
}

impl rockspec::display::AsLuaKV for BuildDependencies<'_> {
    fn display_lua(&self) -> rockspec::display::LuaKV {
        rockspec::display::LuaKV {
            key: "build_dependencies".to_string(),
            value: rockspec::display::LuaValue::List(
                self.0
                    .iter()
                    .map(|req| rockspec::display::LuaValue::String(req.to_string()))
                    .collect(),
            ),
        }
    }
}

/// A luarocks package name, which is always lowercase
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", serde(transparent))]
pub struct Name(String);

impl Name {
    pub fn new(name: String) -> Self {
        Self(name.to_lowercase())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Name::new(String::deserialize(deserializer)?))
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl From<&str> for Name {
    fn from(value: &str) -> Self {
        Self::new(value.into())
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[derive(Debug)]
pub struct List(Vec<Name>);

impl List {
    pub(crate) fn new(package_names: Vec<Name>) -> Self {
        Self(package_names)
    }
}

impl Display for List {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.iter().map(|pkg| pkg.to_string()).join(", ").as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote;

    use crate::package::version::Version;

    #[test]
    fn parse_name() {
        let mut package_name: Name = "neorg".into();
        assert_eq!(package_name.to_string(), "neorg");
        package_name = "LuaFileSystem".into();
        assert_eq!(package_name.to_string(), "luafilesystem");
    }

    #[test]
    fn parse_lua_package() {
        let neorg = Spec::parse("neorg".into(), "1.0.0".into()).unwrap();
        let expected_version = Version::parse("1.0.0").unwrap();
        assert_eq!(neorg.name().to_string(), "neorg");
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = Spec::parse("neorg".into(), "1.0".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = Spec::parse("neorg".into(), "1".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
    }

    #[test]
    fn parse_lua_package_req() {
        let mut package_req = Req::new("foo".into(), Some("1.0.0".into())).unwrap();
        assert!(package_req.matches(&Spec::parse("foo".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&Spec::parse("bar".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&Spec::parse("foo".into(), "2.0.0".into()).unwrap()));
        package_req = Req::new("foo".into(), Some(">= 1.0.0".into())).unwrap();
        assert!(package_req.matches(&Spec::parse("foo".into(), "2.0.0".into()).unwrap()));
        let package_req: Req = "lua >= 5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: Req = "lua>=5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: Req = "toml-edit >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "toml-edit");
        let package_req: Req = "plugin.nvim >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "plugin.nvim");
        let package_req: Req = "lfs".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lfs");
        let package_req: Req = "neorg 1.0.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "neorg");
        let neorg = Spec::parse("neorg".into(), "1.0.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "2.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: Req = "neorg 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg = 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg == 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg &equals; 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg >= 1.0, &lt; 2.0".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg &gt; 1.0, &lt; 2.0".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.11.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "3.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "0.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1".parse().unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1.4".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.4.10".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.4".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1.0.5".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.0.4".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.0.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.0.6".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        // Testing incomplete version constraints
        let package_req: Req = "lua-utils.nvim ~> 1.1-1".parse().unwrap();
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.1.4".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.1.5".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.2-1".into()).unwrap();
        assert!(!package_req.matches(&lua_utils));
    }

    #[test]
    pub fn display_package_req() {
        let package_req = Req::new("foo".into(), Some("1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo==1.0.0");
        let package_req = Req::new("foo".into(), Some(">=1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0");
        let package_req = Req::new("foo".into(), Some(">1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>1.0.0");
        let package_req = Req::new("foo".into(), Some("<=1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo<=1.0.0");
        let package_req = Req::new("foo".into(), Some("<1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo<1.0.0");
        let package_req = Req::new("foo".into(), Some("~>1.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0, <1.1.0");
        let package_req = Req::new("foo".into(), Some(">=1.0.0, <1.1.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0, <1.1.0");
        let package_req = Req::new("foo".into(), None).unwrap();
        assert_eq!(package_req.to_string(), "foo");
        let package_req = Req::new("foo".into(), Some("*".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo");
    }

    #[test]
    pub fn remote_package_type_priorities() {
        let rock_types = vec![
            remote::package::Kind::Binary,
            remote::package::Kind::Src,
            remote::package::Kind::Rockspec,
        ];
        assert_eq!(
            rock_types.into_iter().sorted().collect_vec(),
            vec![
                remote::package::Kind::Src,
                remote::package::Kind::Rockspec,
                remote::package::Kind::Binary,
            ]
        );
    }

    #[test]
    fn package_spec_req_roundtrips() {
        let spec = Spec::new("foo".into(), "1.0.0-1".parse().unwrap());
        let req: Req = spec.clone().into();
        assert_eq!(spec, req.try_into().unwrap());

        let spec = Spec::new("foo".into(), "1.0.0".parse().unwrap());
        let req: Req = spec.clone().into();
        assert_eq!(spec, req.try_into().unwrap());
    }

    #[test]
    fn package_req_spec_roundtrips() {
        let req: Req = "foo@1.0.0".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        assert_eq!(req, spec.into());

        let req: Req = "foo@1.0.0-1".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        assert_eq!(req, spec.into());
    }

    #[test]
    fn package_spec_parses_same_as_name_version() {
        let req: Req = "foo@1.0.0".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        let name: Name = "foo".into();
        let version: Version = "1.0.0".parse().unwrap();
        assert_eq!(spec, Spec::new(name, version));

        let req: Req = "foo@1.0.0-1".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        let name: Name = "foo".into();
        let version: Version = "1.0.0-1".parse().unwrap();
        assert_eq!(spec, Spec::new(name, version));
    }
}
