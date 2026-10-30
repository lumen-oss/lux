use super::super::platform;
use crate::package;
use std::collections::HashMap;

use crate::{rockspec::lua_dependency, rockspec::Format, ROCKSPEC_FUEL_LIMIT};
use miette::Diagnostic;
use ottavino::{Closure, Executor, Fuel};
use ottavino_util::serde::from_value;
use thiserror::Error;

use super::super::{build, deploy, description, external_dependency, test};
use super::parse_lua_tbl_or_default;

/// Deserialized from a Lua `.rockspec`, not yet validated
#[derive(Debug)]
pub struct Partial {
    pub(crate) format: Option<Format>,
    pub(crate) package: Option<package::Name>,
    pub(crate) build: Option<build::internal::Spec>,
    pub(crate) deploy: Option<deploy::Spec>,
    pub(crate) description: Option<description::Spec>,
    pub(crate) supported_platforms: Option<platform::Support>,
    pub(crate) dependencies: Option<Vec<lua_dependency::Spec>>,
    pub(crate) build_dependencies: Option<Vec<lua_dependency::Spec>>,
    pub(crate) external_dependencies: Option<HashMap<String, external_dependency::Spec>>,
    pub(crate) test_dependencies: Option<Vec<lua_dependency::Spec>>,
    pub(crate) test: Option<test::internal::Spec>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("rockspec execution exceeded fuel limit of {ROCKSPEC_FUEL_LIMIT} steps")]
    #[diagnostic(help(
        r#"the rockspec may be too complex or contain too many dependencies.
        Try reducing the number of dependencies or simplifying build instructions."#
    ))]
    FuelLimitExceeded,
    #[error("field '{0}' should not be declared in extra.rockspec")]
    ExtraneousField(String),
    #[error("error while parsing rockspec")]
    Lua(#[from] ottavino::ExternError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Partial {
    pub fn new(rockspec_content: &str) -> Result<Self, Error> {
        let mut lua = ottavino::Lua::core();

        let rockspec = lua.try_enter(|ctx| {
            let closure = Closure::load(ctx, None, rockspec_content.as_bytes())?;

            let executor = Executor::start(ctx, closure.into(), ());

            let output = executor.step(ctx, &mut Fuel::with(ROCKSPEC_FUEL_LIMIT))?;

            if !output {
                return Ok(Err(Error::FuelLimitExceeded));
            }

            let globals = ctx.globals();

            if !matches!(globals.get_value(ctx, "version"), ottavino::Value::Nil) {
                return Ok(Err(Error::ExtraneousField("version".to_string())));
            }
            if !matches!(globals.get_value(ctx, "source"), ottavino::Value::Nil) {
                return Ok(Err(Error::ExtraneousField("source".to_string())));
            }

            let rockspec = Partial {
                format: from_value(globals.get_value(ctx, "rockspec_format")).unwrap_or_default(),
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

        Partial::new(partial_rockspec).unwrap();

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

        let rockspec = Partial::new(full_rockspec).unwrap();

        // No need to verify if the fields were parsed correctly, but worth checking if they were
        // parsed at all.

        assert!(rockspec.format.is_some());
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

        Partial::new(partial_rockspec).unwrap_err();

        let partial_rockspec = r#"
            source = {
                url = "https://example.com",
            }
        "#;

        Partial::new(partial_rockspec).unwrap_err();
    }
}
