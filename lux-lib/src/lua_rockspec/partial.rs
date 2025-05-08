use std::collections::HashMap;

use mlua::{Lua, LuaSerdeExt, UserData, Value};

use crate::{
    lua_rockspec::RockspecFormat,
    package::{PackageName, PackageVersion},
    rockspec::lua_dependency::LuaDependencySpec,
};

use super::{
    parse_lua_tbl_or_default, BuildSpecInternal, DeploySpec, ExternalDependencySpec,
    PlatformSupport, RockDescription, RockSourceInternal, TestSpecInternal,
};

pub struct PartialLuaRockspec {
    pub(crate) rockspec_format: Option<RockspecFormat>,
    pub(crate) package: Option<PackageName>,
    pub(crate) version: Option<PackageVersion>,
    pub(crate) build: Option<BuildSpecInternal>,
    pub(crate) deploy: Option<DeploySpec>,
    pub(crate) description: Option<RockDescription>,
    pub(crate) supported_platforms: Option<PlatformSupport>,
    pub(crate) dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) build_dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) external_dependencies: Option<HashMap<String, ExternalDependencySpec>>,
    pub(crate) test_dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) source: Option<RockSourceInternal>,
    pub(crate) test: Option<TestSpecInternal>,
}

impl UserData for PartialLuaRockspec {}

pub type PartialRockspecError = mlua::Error;

impl PartialLuaRockspec {
    pub fn new(rockspec_content: &str) -> Result<Self, PartialRockspecError> {
        let lua = Lua::new();
        lua.load(rockspec_content).exec()?;

        let globals = lua.globals();

        let rockspec = PartialLuaRockspec {
            rockspec_format: globals.get("rockspec_format").unwrap_or_default(),
            package: globals.get("package").unwrap_or_default(),
            version: globals.get("version").unwrap_or_default(),
            description: parse_lua_tbl_or_default(&lua, "description").unwrap_or_default(),
            supported_platforms: parse_lua_tbl_or_default(&lua, "supported_platforms")
                .unwrap_or_default(),
            dependencies: lua
                .from_value(globals.get("dependencies").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            build_dependencies: lua
                .from_value(globals.get("build_dependencies").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            test_dependencies: lua
                .from_value(globals.get("test_dependencies").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            external_dependencies: lua
                .from_value(globals.get("external_dependencies").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            source: lua
                .from_value(globals.get("source").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            build: lua
                .from_value(globals.get("build").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            test: lua
                .from_value(globals.get("test").unwrap_or(Value::Nil))
                .unwrap_or_default(),
            deploy: lua
                .from_value(globals.get("deploy").unwrap_or(Value::Nil))
                .unwrap_or_default(),
        };

        Ok(rockspec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_partial_rockspec() {
        let partial_rockspec = r#"
            version = "2.0.0"
        "#;

        PartialLuaRockspec::new(partial_rockspec).unwrap();

        let partial_rockspec = r#"
            version = "2.0.0"
            package = "my-package"
        "#;

        PartialLuaRockspec::new(partial_rockspec).unwrap();

        // Whether the partial rockspec format can still support entire rockspecs
        let full_rockspec = r#"
            rockspec_format = "3.0"
            package = "my-package"
            version = "1.0.0"

            description = {
                summary = "A summary",
                detailed = "A detailed description",
                license = "MIT",
                homepage = "https://example.com",
                issues_url = "https://example.com/issues",
                maintainer = "John Doe",
                labels = {"label1", "label2"},
            }

            supported_platforms = {"linux", "!windows"}

            dependencies = {
                "lua 5.1",
                "foo 1.0",
                "bar >=2.0",
            }

            build_dependencies = {
                "baz 1.0",
            }

            external_dependencies = {
                foo = { header = "foo.h" },
                bar = { library = "libbar.so" },
            }

            test_dependencies = {
                "busted 1.0",
            }

            source = {
                url = "https://example.com",
                hash = "sha256-di00mD8txN7rjaVpvxzNbnQsAh6H16zUtJZapH7U4HU=",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }

            test = {
                type = "command",
                script = "test.lua",
                flags = {"foo", "bar"},
            }

            build = {
                type = "builtin",
            }
        "#;

        let rockspec = PartialLuaRockspec::new(full_rockspec).unwrap();

        // No need to verify if the fields were parsed correctly, but worth checking if they were
        // parsed at all.

        assert!(rockspec.rockspec_format.is_some());
        assert!(rockspec.package.is_some());
        assert!(rockspec.version.is_some());
        assert!(rockspec.description.is_some());
        assert!(rockspec.supported_platforms.is_some());
        assert!(rockspec.dependencies.is_some());
        assert!(rockspec.build_dependencies.is_some());
        assert!(rockspec.external_dependencies.is_some());
        assert!(rockspec.test_dependencies.is_some());
        assert!(rockspec.source.is_some());
        assert!(rockspec.build.is_some());
        assert!(rockspec.test.is_some());
    }
}
