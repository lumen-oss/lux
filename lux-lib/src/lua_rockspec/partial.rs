use std::collections::HashMap;

use ottavino::{Closure, Executor, Fuel};
use ottavino_util::serde::from_value;
use thiserror::Error;

use crate::{
    lua_rockspec::RockspecFormat, package::PackageName,
    rockspec::lua_dependency::LuaDependencySpec, ROCKSPEC_FUEL_LIMIT,
};

use super::{
    parse_lua_tbl_or_default, BuildSpecInternal, DeploySpec, ExternalDependencySpec,
    PlatformSupport, RockDescription, TestSpecInternal,
};

#[derive(Debug)]
pub struct PartialLuaRockspec {
    pub(crate) rockspec_format: Option<RockspecFormat>,
    pub(crate) package: Option<PackageName>,
    pub(crate) build: Option<BuildSpecInternal>,
    pub(crate) deploy: Option<DeploySpec>,
    pub(crate) description: Option<RockDescription>,
    pub(crate) supported_platforms: Option<PlatformSupport>,
    pub(crate) dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) build_dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) external_dependencies: Option<HashMap<String, ExternalDependencySpec>>,
    pub(crate) test_dependencies: Option<Vec<LuaDependencySpec>>,
    pub(crate) test: Option<TestSpecInternal>,
}

#[derive(Debug, Error)]
pub enum PartialRockspecError {
    #[error("rockspec execution exceeded fuel limit of {ROCKSPEC_FUEL_LIMIT} steps")]
    FuelLimitExceeded,
    #[error("field `{0}` should not be declared in extra.rockspec")]
    ExtraneousField(String),
    #[error("error while parsing rockspec: {0}")]
    Lua(#[from] ottavino::ExternError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl PartialLuaRockspec {
    pub fn new(rockspec_content: &str) -> Result<Self, PartialRockspecError> {
        let mut lua = ottavino::Lua::core();

        let rockspec = lua.try_enter(|ctx| {
            let closure = Closure::load(ctx, None, rockspec_content.as_bytes())?;

            let executor = Executor::start(ctx, closure.into(), ());

            let output = executor.step(ctx, &mut Fuel::with(ROCKSPEC_FUEL_LIMIT))?;

            if !output {
                return Ok(Err(PartialRockspecError::FuelLimitExceeded));
            }

            let globals = ctx.globals();

            if !matches!(globals.get_value(ctx, "version"), ottavino::Value::Nil) {
                return Ok(Err(PartialRockspecError::ExtraneousField(
                    "version".to_string(),
                )));
            }
            if !matches!(globals.get_value(ctx, "source"), ottavino::Value::Nil) {
                return Ok(Err(PartialRockspecError::ExtraneousField(
                    "source".to_string(),
                )));
            }

            let rockspec = PartialLuaRockspec {
                rockspec_format: from_value(globals.get_value(ctx, "rockspec_format"))
                    .unwrap_or_default(),
                package: from_value(globals.get_value(ctx, "package")).unwrap_or_default(),
                description: parse_lua_tbl_or_default(ctx, "description").unwrap_or_default(),
                supported_platforms: parse_lua_tbl_or_default(ctx, "supported_platforms")
                    .unwrap_or_default(),
                dependencies: from_value(globals.get_value(ctx, "dependencies"))
                    .unwrap_or_default(),
                build_dependencies: from_value(globals.get_value(ctx, "build_dependencies"))
                    .unwrap_or_default(),
                test_dependencies: from_value(globals.get_value(ctx, "test_dependencies"))
                    .unwrap_or_default(),
                external_dependencies: from_value(globals.get_value(ctx, "external_dependencies"))
                    .unwrap_or_default(),
                build: from_value(globals.get_value(ctx, "build")).unwrap_or_default(),
                test: from_value(globals.get_value(ctx, "test")).unwrap_or_default(),
                deploy: from_value(globals.get_value(ctx, "deploy")).unwrap_or_default(),
            };

            Ok(Ok(rockspec))
        })??;

        Ok(rockspec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_partial_rockspec() {
        let partial_rockspec = r#"
            package = "my-package"
        "#;

        PartialLuaRockspec::new(partial_rockspec).unwrap();

        // Whether the partial rockspec format can still support entire rockspecs
        let full_rockspec = r#"
            rockspec_format = "3.0"
            package = "my-package"

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
        assert!(rockspec.description.is_some());
        assert!(rockspec.supported_platforms.is_some());
        assert!(rockspec.dependencies.is_some());
        assert!(rockspec.build_dependencies.is_some());
        assert!(rockspec.external_dependencies.is_some());
        assert!(rockspec.test_dependencies.is_some());
        assert!(rockspec.build.is_some());
        assert!(rockspec.test.is_some());

        // We don't allow version and source in extra.rockspec
        let partial_rockspec = r#"
            version = "2.0.0"
        "#;

        PartialLuaRockspec::new(partial_rockspec).unwrap_err();

        let partial_rockspec = r#"
            source = {
                url = "https://example.com",
            }
        "#;

        PartialLuaRockspec::new(partial_rockspec).unwrap_err();
    }
}
