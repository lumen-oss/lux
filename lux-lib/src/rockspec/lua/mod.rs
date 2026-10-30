pub mod partial;

pub use partial::Partial;

use crate::package;
use std::{collections::HashMap, io, path::PathBuf};

use ottavino::{Closure, Executor, Fuel};
use ottavino_util::serde::from_value;
use serde::{de::DeserializeOwned, Deserialize};

use miette::{Diagnostic, NamedSource, SourceSpan};
use ssri::Integrity;
use thiserror::Error;

use super::{
    build, deploy, description,
    display::{self, AsLuaKV, AsLuaValue},
    external_dependency, platform, source, test, Format, PerPlatform,
};
use crate::{
    hash::HasIntegrity,
    rockspec::{lua_dependency, Rockspec},
    ROCKSPEC_FUEL_LIMIT,
};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("manifest exceeds computational limit of {ROCKSPEC_FUEL_LIMIT} steps")]
    #[diagnostic(
        code(lux_lib::rockspec::lua::fuel_limit_exceeded),
        help(
            r#"the rockspec may be too complex or contain too many dependencies.
try reducing the number of dependencies or simplifying build instructions.
"#
        )
    )]
    FuelLimitExceeded,
    #[error("could not parse rockspec")]
    #[diagnostic(code(lux_lib::rockspec::lua::execution),
        help("check the rockspec or lux.toml for valid syntax and make sure it matches the specification."),
        url("https://lux.lumen-labs.org/reference/lux-toml")
    )]
    ExecutionError {
        cause: Box<String>,
        #[source_code]
        content: NamedSource<String>,
        #[label("this package")]
        pkg_span: SourceSpan,
        #[label("{cause}")]
        err_span: Option<SourceSpan>,
    },
    #[error("{}copy_directories cannot contain the rockspec name", ._0.as_ref().map(|p| format!("{p}: ")).unwrap_or_default())]
    CopyDirectoriesContainRockspecName(Option<String>),
    #[error("cannot create Lua rockspec with off-spec dependency")]
    #[diagnostic(
        code(lux_lib::rockspec::lua::off_spec_dependency),
        url("https://lux.lumen-labs.org/reference/lux-toml#dependencies")
    )]
    OffSpecSpec(package::Name),
    #[error("cannot create Lua rockspec with off-spec build dependency")]
    #[diagnostic(
        code(lux_lib::rockspec::lua::off_spec_build_dependency),
        url("https://lux.lumen-labs.org/reference/lux-toml#dependencies")
    )]
    OffSpecBuildSpec(package::Name),
    #[error("cannot create Lua rockspec with off-spec test dependency")]
    #[diagnostic(
        code(lux_lib::rockspec::lua::off_spec_test_dependency),
        url("https://lux.lumen-labs.org/reference/lux-toml#dependencies")
    )]
    OffSpecTestSpec(package::Name),
}

impl Error {
    fn from_ottavino(cause: ottavino::ExternError, rockspec_content: &str) -> Self {
        let msg = match &cause {
            ottavino::ExternError::Runtime(rt) => rt.to_string(),
            _ => cause.to_string(),
        };
        let field = msg
            .strip_prefix("invalid rockspec field '")
            .and_then(|rest| rest.split('\'').next())
            .map(|f| f.to_owned());
        let err_span = field.as_deref().and_then(|field| {
            let span = find_key_span(rockspec_content, field);
            if span.is_empty() {
                None
            } else {
                Some(span)
            }
        });
        Error::ExecutionError {
            cause: Box::new(msg),
            content: NamedSource::new("rockspec", rockspec_content.to_string()),
            pkg_span: find_value_span(rockspec_content, "package"),
            err_span,
        }
    }
}

/// This error can be returned within `ottavino::Lua::try_enter`.
/// ottavino wraps it in an [`ottavino::ExternError`], which Lux wraps inn
/// a [`Error::ExecutionError`]
#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
enum ParseLuaRockspecFieldError {
    #[error("invalid rockspec field '{field}': {source}")]
    Deserialize {
        field: String,
        source: Box<ottavino_util::serde::de::Error>,
    },
    #[error(
        r#"invalid rockspec field '{field}'.
Failed to parse a table field: {source}"#
    )]
    DeserializeTable {
        field: String,
        source: Box<ottavino_util::serde::de::Error>,
    },
    #[error("invalid rockspec field '{field}'. expected a table, but got: {invalid_type}")]
    NotATable { field: String, invalid_type: String },
}

/// RockSpec for a local rock installation, deserialized from a `.rockspec` file
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Local {
    /// The file format version. Example: "1.0"
    format: Option<Format>,
    /// The name of the package. Example: "luasocket"
    package: package::Name,
    /// The version of the package, plus a suffix indicating the revision of the rockspec. Example: "2.0.1-1"
    version: package::Version,
    description: description::Spec,
    supported_platforms: platform::Support,
    /// The Lua version requirement for this rock
    lua: package::VersionReq,
    dependencies: PerPlatform<Vec<lua_dependency::Spec>>,
    build_dependencies: PerPlatform<Vec<lua_dependency::Spec>>,
    external_dependencies: PerPlatform<HashMap<String, external_dependency::Spec>>,
    test_dependencies: PerPlatform<Vec<lua_dependency::Spec>>,
    build: PerPlatform<build::Spec>,
    source: PerPlatform<source::Remote>,
    test: PerPlatform<test::Spec>,
    deploy: PerPlatform<deploy::Spec>,
    /// The original content of this rockspec, needed by luarocks
    raw_content: String,
}

trait HasRockspecKey<'gc> {
    fn get_rockspec_key<V: Deserialize<'gc>>(
        &self,
        ctx: ottavino::Context<'gc>,
        key: &str,
    ) -> Result<V, ParseLuaRockspecFieldError>;
}

impl<'gc> HasRockspecKey<'gc> for ottavino::Table<'gc> {
    fn get_rockspec_key<V: Deserialize<'gc>>(
        &self,
        ctx: ottavino::Context<'gc>,
        key: &str,
    ) -> Result<V, ParseLuaRockspecFieldError> {
        let value = self.get_value(ctx, key.to_string());
        let is_table = matches!(&value, ottavino::Value::Table(_));
        from_value(value).map_err(|source| {
            if is_table {
                ParseLuaRockspecFieldError::DeserializeTable {
                    field: key.into(),
                    source: Box::new(source),
                }
            } else {
                ParseLuaRockspecFieldError::Deserialize {
                    field: key.into(),
                    source: Box::new(source),
                }
            }
        })
    }
}

impl Local {
    /// Builds a [`Local`] from an already-parsed project.
    pub(crate) fn from_project_toml(project: &super::toml::Local, raw_content: String) -> Self {
        use crate::rockspec::Rockspec;

        Self {
            format: project.format().clone(),
            package: project.package().clone(),
            version: project.version().clone(),
            description: project.description().clone(),
            supported_platforms: project.supported_platforms().clone(),
            lua: project.lua().clone(),
            dependencies: project.dependencies().clone(),
            build_dependencies: project.build_dependencies().clone(),
            external_dependencies: project.external_dependencies().clone(),
            test_dependencies: project.test_dependencies().clone(),
            build: project.build().clone(),
            source: project.source().clone(),
            test: project.test().clone(),
            deploy: project.deploy().clone(),
            raw_content,
        }
    }

    /// Wraps this rockspec into a [`Remote`], overriding its source
    /// with the given `source`.
    pub fn into_remote_with_source(self, source: PerPlatform<source::Remote>) -> Remote {
        Remote {
            local: self,
            source,
        }
    }

    pub fn new(rockspec_content: &str, project_root: PathBuf) -> Result<Self, Error> {
        let mut lua = ottavino::Lua::core();

        let rockspec = lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, None, rockspec_content.as_bytes())?;

                let executor = Executor::start(ctx, closure.into(), ());

                let output = executor.step(ctx, &mut Fuel::with(ROCKSPEC_FUEL_LIMIT))?;

                if !output {
                    return Ok(Err(Error::FuelLimitExceeded));
                }

                let globals = ctx.globals();

                let dependencies: PerPlatform<Vec<lua_dependency::Spec>> =
                    globals.get_rockspec_key(ctx, "dependencies")?;

                let lua_version_req = dependencies
                    .current_platform()
                    .iter()
                    .find(|dep| dep.name().to_string() == "lua")
                    .cloned()
                    .map(|dep| dep.version_req().clone())
                    .unwrap_or(package::VersionReq::Any);

                fn strip_lua(
                    dependencies: PerPlatform<Vec<lua_dependency::Spec>>,
                ) -> PerPlatform<Vec<lua_dependency::Spec>> {
                    dependencies.map(|deps| {
                        deps.iter()
                            .filter(|dep| dep.name().to_string() != "lua")
                            .cloned()
                            .collect()
                    })
                }

                let build_dependencies: PerPlatform<Vec<lua_dependency::Spec>> =
                    globals.get_rockspec_key(ctx, "build_dependencies")?;

                let test_dependencies: PerPlatform<Vec<lua_dependency::Spec>> =
                    globals.get_rockspec_key(ctx, "test_dependencies")?;

                let source: PerPlatform<source::Remote> = match globals.get_value(ctx, "source") {
                    ottavino::Value::Nil => {
                        PerPlatform::new(source::Spec::File(project_root).into())
                    }
                    value => from_value(value).map_err(|source| {
                        ParseLuaRockspecFieldError::Deserialize {
                            field: "source".into(),
                            source: Box::new(source),
                        }
                    })?,
                };

                let rockspec = Local {
                    format: globals.get_rockspec_key(ctx, "rockspec_format")?,
                    package: globals.get_rockspec_key(ctx, "package")?,
                    version: globals.get_rockspec_key(ctx, "version")?,
                    description: parse_lua_tbl_or_default(ctx, "description")?,
                    supported_platforms: parse_lua_tbl_or_default(ctx, "supported_platforms")?,
                    lua: lua_version_req,
                    dependencies: strip_lua(dependencies),
                    build_dependencies: strip_lua(build_dependencies),
                    test_dependencies: strip_lua(test_dependencies),
                    external_dependencies: globals
                        .get_rockspec_key(ctx, "external_dependencies")?,
                    build: globals.get_rockspec_key(ctx, "build")?,
                    test: globals.get_rockspec_key(ctx, "test")?,
                    deploy: globals.get_rockspec_key(ctx, "deploy")?,
                    raw_content: rockspec_content.into(),

                    source,
                };

                Ok(Ok(rockspec))
            })
            .map_err(|cause| Error::from_ottavino(cause, rockspec_content))??;

        let rockspec_file_name = format!("{}-{}.rockspec", rockspec.package(), rockspec.version());

        if rockspec
            .build()
            .default
            .copy_directories
            .contains(&PathBuf::from(&rockspec_file_name))
        {
            return Err(Error::CopyDirectoriesContainRockspecName(None));
        }

        for (platform, build_override) in &rockspec.build().per_platform {
            if build_override
                .copy_directories
                .contains(&PathBuf::from(&rockspec_file_name))
            {
                return Err(Error::CopyDirectoriesContainRockspecName(Some(
                    platform.to_string(),
                )));
            }
        }
        Ok(rockspec)
    }
}

impl Rockspec for Local {
    fn package(&self) -> &package::Name {
        &self.package
    }

    fn version(&self) -> &package::Version {
        &self.version
    }

    fn description(&self) -> &description::Spec {
        &self.description
    }

    fn supported_platforms(&self) -> &platform::Support {
        &self.supported_platforms
    }

    fn lua(&self) -> &package::VersionReq {
        &self.lua
    }

    fn dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        &self.dependencies
    }

    fn build_dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        &self.build_dependencies
    }

    fn external_dependencies(&self) -> &PerPlatform<HashMap<String, external_dependency::Spec>> {
        &self.external_dependencies
    }

    fn test_dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        &self.test_dependencies
    }

    fn build(&self) -> &PerPlatform<build::Spec> {
        &self.build
    }

    fn test(&self) -> &PerPlatform<test::Spec> {
        &self.test
    }

    fn source(&self) -> &PerPlatform<source::Remote> {
        &self.source
    }

    fn deploy(&self) -> &PerPlatform<deploy::Spec> {
        &self.deploy
    }

    fn build_mut(&mut self) -> &mut PerPlatform<build::Spec> {
        &mut self.build
    }

    fn test_mut(&mut self) -> &mut PerPlatform<test::Spec> {
        &mut self.test
    }

    fn source_mut(&mut self) -> &mut PerPlatform<source::Remote> {
        &mut self.source
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<deploy::Spec> {
        &mut self.deploy
    }

    fn format(&self) -> &Option<Format> {
        &self.format
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, super::Error> {
        Ok(self.raw_content.clone())
    }
}

impl HasIntegrity for Local {
    async fn hash(&self) -> io::Result<Integrity> {
        Ok(Integrity::from(&self.raw_content))
    }
}

/// RockSpec for a remote rock, deserialized from a `.rockspec` file
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Remote {
    local: Local,
    source: PerPlatform<source::Remote>,
}

impl Remote {
    pub fn new(rockspec_content: &str) -> Result<Self, Error> {
        let mut lua = ottavino::Lua::core();

        let source = lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, None, rockspec_content.as_bytes())?;

                let executor = Executor::start(ctx, closure.into(), ());

                let output = executor.step(ctx, &mut Fuel::with(ROCKSPEC_FUEL_LIMIT))?;

                if !output {
                    return Ok(Err(Error::FuelLimitExceeded));
                }

                let globals = ctx.globals();

                let source: PerPlatform<source::Remote> =
                    globals.get_rockspec_key(ctx, "source")?;

                Ok(Ok(source))
            })
            .map_err(|cause| Error::from_ottavino(cause, rockspec_content))??;

        Ok(Remote {
            // NOTE: Calling this outside of `lua.try_enter` reduces error message repetition
            local: Local::new(rockspec_content, PathBuf::new())?,
            source,
        })
    }

    /// Builds a rockspec from a source spec and dependencies.
    ///
    /// With no modules, the rockspec uses the `source` build backend.
    /// Otherwise, it uses the `builtin` build backend with the given modules.
    pub(crate) fn from_source_spec(
        package_spec: package::Spec,
        source_spec: source::Spec,
        dependencies: Vec<lua_dependency::Spec>,
        modules: HashMap<build::backend::Module, build::backend::ModuleSpec>,
    ) -> Self {
        let version = package_spec.version().clone();
        let rockspec_format = Format::default();
        let (backend, build_lua) = if modules.is_empty() {
            (
                build::backend::Spec::Source,
                r#"type = "source""#.to_string(),
            )
        } else {
            let modules_lua = modules_lua(&modules);
            (
                build::backend::Spec::Builtin(build::backend::Builtin { modules }),
                format!("type = \"builtin\",\n  modules = {{\n{modules_lua}\n  }}"),
            )
        };
        let build_spec = build::Spec {
            backend: Some(backend),
            ..Default::default()
        };
        let raw_content = rockspec_content(&package_spec, &source_spec, &build_lua);
        let source: source::Remote = source_spec.into();

        let local = Local {
            format: Some(rockspec_format),
            package: package_spec.name().clone(),
            version,
            description: description::Spec::default(),
            supported_platforms: platform::Support::default(),
            lua: package::VersionReq::Any,
            dependencies: PerPlatform::new(dependencies),
            build_dependencies: PerPlatform::default(),
            external_dependencies: PerPlatform::default(),
            test_dependencies: PerPlatform::default(),
            build: PerPlatform::new(build_spec),
            source: PerPlatform::new(source.clone()),
            test: PerPlatform::default(),
            deploy: PerPlatform::default(),
            raw_content,
        };
        Self {
            local,
            source: PerPlatform::new(source),
        }
    }
}

fn rockspec_content(
    package_spec: &package::Spec,
    source_spec: &source::Spec,
    build_lua: &str,
) -> String {
    format!(
        r#"
rockspec_format = "{}"
package = "{}"
version = "{}"
{}
build = {{
{}
}}"#,
        Format::default(),
        package_spec.name(),
        package_spec.version(),
        source_spec.display_lua(),
        build_lua,
    )
}

fn modules_lua(modules: &HashMap<build::backend::Module, build::backend::ModuleSpec>) -> String {
    let mut entries: Vec<_> = modules.iter().collect();
    entries.sort_by_key(|(module, _)| module.as_str());
    display::LuaValue::Table(
        entries
            .into_iter()
            .map(|(module, spec)| display::LuaKV {
                key: module.to_string(),
                value: spec.display_lua_value(),
            })
            .collect(),
    )
    .to_string()
}

impl Rockspec for Remote {
    fn package(&self) -> &package::Name {
        self.local.package()
    }

    fn version(&self) -> &package::Version {
        self.local.version()
    }

    fn description(&self) -> &description::Spec {
        self.local.description()
    }

    fn supported_platforms(&self) -> &platform::Support {
        self.local.supported_platforms()
    }

    fn lua(&self) -> &package::VersionReq {
        self.local.lua()
    }

    fn dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        self.local.dependencies()
    }

    fn build_dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        match self.format() {
            // Rockspec formats < 3.0 don't support `build_dependencies`,
            // so we have to return regular dependencies if the build backend might need to use them.
            Some(Format::_1_0 | Format::_2_0)
                if self
                    .build()
                    .current_platform()
                    .backend
                    .as_ref()
                    .is_some_and(|build_backend| build_backend.can_use_build_dependencies()) =>
            {
                self.local.dependencies()
            }
            _ => self.local.build_dependencies(),
        }
    }

    fn external_dependencies(&self) -> &PerPlatform<HashMap<String, external_dependency::Spec>> {
        self.local.external_dependencies()
    }

    fn test_dependencies(&self) -> &PerPlatform<Vec<lua_dependency::Spec>> {
        self.local.test_dependencies()
    }

    fn build(&self) -> &PerPlatform<build::Spec> {
        self.local.build()
    }

    fn test(&self) -> &PerPlatform<test::Spec> {
        self.local.test()
    }

    fn source(&self) -> &PerPlatform<source::Remote> {
        &self.source
    }

    fn deploy(&self) -> &PerPlatform<deploy::Spec> {
        self.local.deploy()
    }

    fn build_mut(&mut self) -> &mut PerPlatform<build::Spec> {
        self.local.build_mut()
    }

    fn test_mut(&mut self) -> &mut PerPlatform<test::Spec> {
        self.local.test_mut()
    }

    fn source_mut(&mut self) -> &mut PerPlatform<source::Remote> {
        &mut self.source
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<deploy::Spec> {
        self.local.deploy_mut()
    }

    fn format(&self) -> &Option<Format> {
        self.local.format()
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, super::Error> {
        Ok(self.local.raw_content.clone())
    }
}

impl HasIntegrity for Remote {
    async fn hash(&self) -> io::Result<Integrity> {
        Ok(Integrity::from(&self.local.raw_content))
    }
}

fn find_key_span(rockspec_content: &str, key: &str) -> SourceSpan {
    rockspec_content
        .match_indices(key)
        .find(|&(start, _)| {
            let end = start + key.len();
            let prefix_ok =
                start == 0 || !rockspec_content.as_bytes()[start - 1].is_ascii_alphanumeric();
            let suffix_ok = rockspec_content
                .get(end..)
                .is_some_and(|s| s.starts_with('=') || s.trim_start().starts_with('='));
            prefix_ok && suffix_ok
        })
        .map(|(start, _)| (start..start + key.len()).into())
        .unwrap_or((0..0).into())
}

fn find_value_span(rockspec_content: &str, key: &str) -> SourceSpan {
    let key = find_key_span(rockspec_content, key);
    if key.is_empty() {
        return key;
    }
    let key_offset = key.offset();
    let key_end = key_offset + key.len();
    let eq_start = key_end + rockspec_content[key_end..].find('=').unwrap_or(0);
    let value_offset = eq_start + 1;
    let line_end = rockspec_content[value_offset..]
        .find('\n')
        .map(|i| value_offset + i)
        .unwrap_or(rockspec_content.len());
    let trimmed = &rockspec_content[value_offset..line_end];
    let start = value_offset + (trimmed.len() - trimmed.trim_start().len());
    let end = line_end - (trimmed.len() - trimmed.trim_end().len());
    (start..end).into()
}

fn parse_lua_tbl_or_default<T>(
    ctx: ottavino::Context<'_>,
    field: &str,
) -> Result<T, ParseLuaRockspecFieldError>
where
    T: Default,
    T: DeserializeOwned,
{
    let ret = match ctx.globals().get_value(ctx, field.to_string()) {
        ottavino::Value::Nil => T::default(),
        value @ ottavino::Value::Table(_) => {
            from_value(value).map_err(|source| ParseLuaRockspecFieldError::DeserializeTable {
                field: field.to_string(),
                source: Box::new(source),
            })?
        }
        value => Err(ParseLuaRockspecFieldError::NotATable {
            field: field.to_string(),
            invalid_type: value.type_name().to_string(),
        })?,
    };
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use crate::git;
    use url::Url;

    use super::*;

    #[test]
    pub fn parse_rockspec() {
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.format, Some(Format::_1_0));
        assert_eq!(rockspec.local.package, "foo".into());
        assert_eq!(rockspec.local.version, "1.0.0-1".parse().unwrap());
        assert_eq!(rockspec.local.description, description::Spec::default());

        let rockspec_content = "
        package = 'bar'\n
        version = '2.0.0-1'\n
        description = {}\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.format, None);
        assert_eq!(rockspec.local.package, "bar".into());
        assert_eq!(rockspec.local.version, "2.0.0-1".parse().unwrap());
        assert_eq!(rockspec.local.description, description::Spec::default());

        let rockspec_content = "
        package = 'rocks.nvim'\n
        version = '3.0.0-1'\n
        description = {\n
            summary = 'some summary',
            detailed = 'some detailed description',
            license = 'MIT',
            homepage = 'https://github.com/lumen-oss/rocks.nvim',
            issues_url = 'https://github.com/lumen-oss/rocks.nvim/issues',
            maintainer = 'Lumen Labs',
        }\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = description::Spec {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim/issues").unwrap()),
            maintainer: Some("Lumen Labs".into()),
            labels: Vec::new(),
        };
        assert_eq!(rockspec.local.description, expected_description);

        let rockspec_content = "
        package = 'rocks.nvim'\n
        version = '3.0.0-1'\n
        description = {\n
            summary = 'some summary',
            detailed = 'some detailed description',
            license = 'MIT',
            homepage = 'https://github.com/lumen-oss/rocks.nvim',
            issues_url = 'https://github.com/lumen-oss/rocks.nvim/issues',
            maintainer = 'Lumen Labs',
            labels = {},
        }\n
        external_dependencies = { FOO = { library = 'foo' } }\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = description::Spec {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim/issues").unwrap()),
            maintainer: Some("Lumen Labs".into()),
            labels: Vec::new(),
        };
        assert_eq!(rockspec.local.description, expected_description);
        assert_eq!(
            *rockspec
                .local
                .external_dependencies
                .default
                .get("FOO")
                .unwrap(),
            external_dependency::Spec {
                library: Some("foo".into()),
                header: None
            }
        );

        let rockspec_content = "
        package = 'rocks.nvim'\n
        version = '3.0.0-1'\n
        description = {\n
            summary = 'some summary',
            detailed = 'some detailed description',
            license = 'MIT',
            homepage = 'https://github.com/lumen-oss/rocks.nvim',
            issues_url = 'https://github.com/lumen-oss/rocks.nvim/issues',
            maintainer = 'Lumen Labs',
            labels = { 'package management', },
        }\n
        supported_platforms = { 'unix', '!windows' }\n
        dependencies = { 'neorg ~> 6' }\n
        build_dependencies = { 'foo' }\n
        external_dependencies = { FOO = { header = 'foo.h' } }\n
        test_dependencies = { 'busted >= 2.0.0' }\n
        source = {\n
            url = 'git+https://github.com/lumen-oss/rocks.nvim',\n
            hash = 'sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek=',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = description::Spec {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim/issues").unwrap()),
            maintainer: Some("Lumen Labs".into()),
            labels: vec!["package management".into()],
        };
        assert_eq!(rockspec.local.description, expected_description);
        assert!(rockspec
            .local
            .supported_platforms
            .is_supported(&platform::Identifier::Unix));
        assert!(!rockspec
            .local
            .supported_platforms
            .is_supported(&platform::Identifier::Windows));
        let neorg = package::Spec::parse("neorg".into(), "6.0.0".into()).unwrap();
        assert!(rockspec
            .local
            .dependencies
            .default
            .into_iter()
            .any(|dep| dep.matches(&neorg)));
        let foo = package::Spec::parse("foo".into(), "1.0.0".into()).unwrap();
        assert!(rockspec
            .local
            .build_dependencies
            .default
            .into_iter()
            .any(|dep| dep.matches(&foo)));
        let busted = package::Spec::parse("busted".into(), "2.2.0".into()).unwrap();
        assert_eq!(
            *rockspec
                .local
                .external_dependencies
                .default
                .get("FOO")
                .unwrap(),
            external_dependency::Spec {
                header: Some("foo.h".into()),
                library: None
            }
        );
        assert!(rockspec
            .local
            .test_dependencies
            .default
            .into_iter()
            .any(|dep| dep.matches(&busted)));

        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/owner/example-project/',\n
            branch = 'bar',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            source::Spec::Git(git::Source {
                url: "https://hub.com/owner/example-project/".parse().unwrap(),
                git_ref: Some(git::Ref::Branch("bar".into()))
            })
        );
        assert_eq!(rockspec.local.test, PerPlatform::default());
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/owner/example-project/',\n
            tag = 'bar',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            source::Spec::Git(git::Source {
                url: "https://hub.com/owner/example-project/".parse().unwrap(),
                git_ref: Some(git::Ref::Tag("bar".into()))
            })
        );
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/owner/example-project/',\n
            branch = 'bar',\n
            tag = 'baz',\n
        }\n
        "
        .to_string();
        let _rockspec = Remote::new(&rockspec_content).unwrap_err();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/owner/example-project/',\n
            tag = 'bar',\n
            file = 'foo.tar.gz',\n
        }\n
        build = {\n
            install = {\n
                conf = {['foo.bar'] = 'config/bar.toml'},\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.archive_name,
            Some("foo.tar.gz".into())
        );
        let foo_bar_path = rockspec
            .local
            .build
            .default
            .install
            .conf
            .get("foo.bar")
            .unwrap();
        assert_eq!(*foo_bar_path, PathBuf::from("config/bar.toml"));
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
        }\n
        build = {\n
            install = {\n
                lua = {\n
                    'foo.lua',\n
                    ['foo.bar'] = 'src/bar.lua',\n
                },\n
                bin = {['foo.bar'] = 'bin/bar'},\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert!(matches!(
            rockspec.local.build.default.backend,
            Some(build::backend::Spec::Builtin { .. })
        ));
        let install_lua_spec = rockspec.local.build.default.install.lua;
        let foo_bar_path = install_lua_spec
            .get(&build::backend::Module::from_str("foo.bar").unwrap())
            .unwrap();
        assert_eq!(*foo_bar_path, PathBuf::from("src/bar.lua"));
        let foo_path = install_lua_spec
            .get(&build::backend::Module::from_str("foo").unwrap())
            .unwrap();
        assert_eq!(*foo_path, PathBuf::from("foo.lua"));
        let foo_bar_path = rockspec
            .local
            .build
            .default
            .install
            .bin
            .get("foo.bar")
            .unwrap();
        assert_eq!(*foo_bar_path, PathBuf::from("bin/bar"));
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/',\n
        }\n
        build = {\n
            copy_directories = { 'lua' },\n
        }\n
        "
        .to_string();
        let _rockspec = Remote::new(&rockspec_content).unwrap_err();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/',\n
        }\n
        build = {\n
            copy_directories = { 'lib' },\n
        }\n
        "
        .to_string();
        let _rockspec = Remote::new(&rockspec_content).unwrap_err();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/',\n
        }\n
        build = {\n
            copy_directories = { 'rock_manifest' },\n
        }\n
        "
        .to_string();
        let _rockspec = Remote::new(&rockspec_content).unwrap_err();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
            dir = 'baz',\n
        }\n
        build = {\n
            type = 'make',\n
            install = {\n
                lib = {['foo.so'] = 'lib/bar.so'},\n
            },\n
            copy_directories = {\n
                'plugin',\n
                'ftplugin',\n
            },\n
            patches = {\n
                ['lua51-support.diff'] = [[\n
                    --- before.c\n
                    +++ path/to/after.c\n
                ]],\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.source.default.unpack_dir, Some("baz".into()));
        assert_eq!(
            rockspec.local.build.default.backend,
            Some(build::backend::Spec::Make(build::backend::Make::default()))
        );
        let foo_bar_path = rockspec
            .local
            .build
            .default
            .install
            .lib
            .get("foo.so")
            .unwrap();
        assert_eq!(*foo_bar_path, PathBuf::from("lib/bar.so"));
        let copy_directories = rockspec.local.build.default.copy_directories;
        assert_eq!(
            copy_directories,
            vec![PathBuf::from("plugin"), PathBuf::from("ftplugin")]
        );
        let patches = rockspec.local.build.default.patches;
        let _patch = patches.get(&PathBuf::from("lua51-support.diff")).unwrap();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
        }\n
        build = {\n
            type = 'cmake',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.build.default.backend,
            Some(build::backend::Spec::CMake(build::backend::CMake::default()))
        );
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
        }\n
        build = {\n
            type = 'command',\n
            build_command = 'foo',\n
            install_command = 'bar',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert!(matches!(
            rockspec.local.build.default.backend,
            Some(build::backend::Spec::Command(
                build::backend::Command { .. }
            ))
        ));
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
        }\n
        build = {\n
            type = 'command',\n
            install_command = 'foo',\n
        }\n
        "
        .to_string();
        Remote::new(&rockspec_content).unwrap();
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/foo.zip',\n
        }\n
        build = {\n
            type = 'command',\n
            build_command = 'foo',\n
        }\n
        "
        .to_string();
        Remote::new(&rockspec_content).unwrap();
        // platform overrides
        let rockspec_content = "
        package = 'rocks'\n
        version = '3.0.0-1'\n
        dependencies = {\n
          'neorg ~> 6',\n
          'toml-edit ~> 1',\n
          platforms = {\n
            windows = {\n
              'neorg = 5.0.0',\n
              'toml = 1.0.0',\n
            },\n
            unix = {\n
              'neorg = 5.0.0',\n
            },\n
            linux = {\n
              'toml = 1.0.0',\n
            },\n
          },\n
        }\n
        source = {\n
            url = 'git+https://github.com/lumen-oss/rocks.nvim',\n
            hash = 'sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek=',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        let neorg_override = package::Spec::parse("neorg".into(), "5.0.0".into()).unwrap();
        let toml_edit = package::Spec::parse("toml-edit".into(), "1.0.0".into()).unwrap();
        let toml = package::Spec::parse("toml".into(), "1.0.0".into()).unwrap();
        assert_eq!(rockspec.local.dependencies.default.len(), 2);
        let per_platform = &rockspec.local.dependencies.per_platform;
        assert_eq!(
            per_platform
                .get(&platform::Identifier::Windows)
                .unwrap()
                .iter()
                .filter(|dep| dep.matches(&neorg_override)
                    || dep.matches(&toml_edit)
                    || dep.matches(&toml))
                .count(),
            3
        );
        assert_eq!(
            per_platform
                .get(&platform::Identifier::Unix)
                .unwrap()
                .iter()
                .filter(|dep| dep.matches(&neorg_override)
                    || dep.matches(&toml_edit)
                    || dep.matches(&toml))
                .count(),
            2
        );
        assert_eq!(
            per_platform
                .get(&platform::Identifier::Linux)
                .unwrap()
                .iter()
                .filter(|dep| dep.matches(&neorg_override)
                    || dep.matches(&toml_edit)
                    || dep.matches(&toml))
                .count(),
            3
        );
        let rockspec_content = "
        package = 'rocks'\n
        version = '3.0.0-1'\n
        external_dependencies = {\n
            FOO = { library = 'foo' },\n
            platforms = {\n
              windows = {\n
                FOO = { library = 'foo.dll' },\n
              },\n
              unix = {\n
                BAR = { header = 'bar.h' },\n
              },\n
              linux = {\n
                FOO = { library = 'foo.so' },\n
              },\n
            },\n
        }\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            *rockspec
                .local
                .external_dependencies
                .default
                .get("FOO")
                .unwrap(),
            external_dependency::Spec {
                library: Some("foo".into()),
                header: None
            }
        );
        let per_platform = rockspec.local.external_dependencies.per_platform;
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Windows)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            external_dependency::Spec {
                library: Some("foo.dll".into()),
                header: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Unix)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            external_dependency::Spec {
                library: Some("foo".into()),
                header: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Unix)
                .and_then(|it| it.get("BAR"))
                .unwrap(),
            external_dependency::Spec {
                header: Some("bar.h".into()),
                library: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Linux)
                .and_then(|it| it.get("BAR"))
                .unwrap(),
            external_dependency::Spec {
                header: Some("bar.h".into()),
                library: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Linux)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            external_dependency::Spec {
                library: Some("foo.so".into()),
                header: None
            }
        );
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'git+https://hub.com/example-project/.git',\n
            branch = 'bar',\n
            platforms = {\n
                macosx = {\n
                    branch = 'mac',\n
                },\n
                windows = {\n
                    url = 'git+https://winhub.com/example-project/.git',\n
                    branch = 'win',\n
                },\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            source::Spec::Git(git::Source {
                url: "https://hub.com/example-project/.git".parse().unwrap(),
                git_ref: Some(git::Ref::Branch("bar".into()))
            })
        );
        assert_eq!(
            rockspec
                .source
                .per_platform
                .get(&platform::Identifier::MacOSX)
                .map(|it| it.source_spec.clone())
                .unwrap(),
            source::Spec::Git(git::Source {
                url: "https://hub.com/example-project/.git".parse().unwrap(),
                git_ref: Some(git::Ref::Branch("mac".into()))
            })
        );
        assert_eq!(
            rockspec
                .source
                .per_platform
                .get(&platform::Identifier::Windows)
                .map(|it| it.source_spec.clone())
                .unwrap(),
            source::Spec::Git(git::Source {
                url: "https://winhub.com/example-project/.git".parse().unwrap(),
                git_ref: Some(git::Ref::Branch("win".into()))
            })
        );
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = { url = 'git+https://hub.com/example-project/foo.zip' }\n
        build = {\n
            type = 'make',\n
            install = {\n
                lib = {['foo.bar'] = 'lib/bar.so'},\n
            },\n
            copy_directories = { 'plugin' },\n
            platforms = {\n
                unix = {\n
                    copy_directories = { 'ftplugin' },\n
                },\n
                linux = {\n
                    copy_directories = { 'foo' },\n
                },\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        let per_platform = rockspec.local.build.per_platform;
        let unix = per_platform.get(&platform::Identifier::Unix).unwrap();
        assert_eq!(
            unix.copy_directories,
            vec![PathBuf::from("plugin"), PathBuf::from("ftplugin")]
        );
        let linux = per_platform.get(&platform::Identifier::Linux).unwrap();
        assert_eq!(
            linux.copy_directories,
            vec![
                PathBuf::from("plugin"),
                PathBuf::from("foo"),
                PathBuf::from("ftplugin")
            ]
        );
        let rockspec_content = "
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = { url = 'git+https://hub.com/example-project/foo.zip' }\n
        build = {\n
            type = 'builtin',\n
            modules = {\n
                cjson = {\n
                    sources = { 'lua_cjson.c', 'strbuf.c', 'fpconv.c' },\n
                }\n
            },\n
            platforms = {\n
                win32 = { modules = { cjson = { defines = {\n
                    'DISABLE_INVALID_NUMBERS', 'USE_INTERNAL_ISINF'\n
                } } } }\n
            },\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        let win32 = rockspec.local.build.get(&platform::Identifier::Windows);
        assert_eq!(
            win32.backend,
            Some(build::backend::Spec::Builtin(build::backend::Builtin {
                modules: vec![(
                    build::backend::Module::from_str("cjson").unwrap(),
                    build::backend::ModuleSpec::ModulePaths(build::backend::ModulePaths {
                        sources: vec!["lua_cjson.c".into(), "strbuf.c".into(), "fpconv.c".into()],
                        libraries: Vec::default(),
                        defines: vec![
                            ("DISABLE_INVALID_NUMBERS".into(), None),
                            ("USE_INTERNAL_ISINF".into(), None)
                        ],
                        incdirs: Vec::default(),
                        libdirs: Vec::default(),
                    })
                )]
                .into_iter()
                .collect()
            }))
        );
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        deploy = {\n
            wrap_bin_scripts = false,\n
        }\n
        source = { url = 'git+https://hub.com/example-project/foo.zip' }\n
        ";
        let rockspec = Remote::new(rockspec_content).unwrap();
        let deploy_spec = &rockspec.deploy().current_platform();
        assert!(!deploy_spec.wrap_bin_scripts);
    }

    #[test]
    pub fn parse_scm_rockspec() {
        let rockspec_content = "
        package = 'foo'\n
        version = 'scm-1'\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.package, "foo".into());
        assert_eq!(rockspec.local.version, "scm-1".parse().unwrap());
    }

    #[test]
    pub fn regression_luasystem() {
        let rockspec_content = String::from_utf8(
            std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/luasystem-0.4.4-1.rockspec"),
            )
            .unwrap(),
        )
        .unwrap();
        let rockspec = Remote::new(&rockspec_content).unwrap();
        let build_spec = rockspec.local.build.current_platform();
        assert!(matches!(
            build_spec.backend,
            Some(build::backend::Spec::Builtin { .. })
        ));
        if let Some(build::backend::Spec::Builtin(build::backend::Builtin { modules })) =
            &build_spec.backend
        {
            assert_eq!(
                modules.get(&build::backend::Module::from_str("system.init").unwrap()),
                Some(&build::backend::ModuleSpec::SourcePath(
                    "system/init.lua".into()
                ))
            );
            assert_eq!(
                modules.get(&build::backend::Module::from_str("system.core").unwrap()),
                Some(&build::backend::ModuleSpec::ModulePaths(
                    build::backend::ModulePaths {
                        sources: vec![
                            "src/core.c".into(),
                            "src/compat.c".into(),
                            "src/time.c".into(),
                            "src/environment.c".into(),
                            "src/random.c".into(),
                            "src/term.c".into(),
                            "src/bitflags.c".into(),
                            "src/wcwidth.c".into(),
                        ],
                        defines: luasystem_expected_defines(),
                        libraries: luasystem_expected_libraries(),
                        incdirs: luasystem_expected_incdirs(),
                        libdirs: luasystem_expected_libdirs(),
                    }
                ))
            );
        }
        if let Some(build::backend::Spec::Builtin(build::backend::Builtin { modules })) = &rockspec
            .local
            .build
            .get(&platform::Identifier::Windows)
            .backend
        {
            if let build::backend::ModuleSpec::ModulePaths(paths) = modules
                .get(&build::backend::Module::from_str("system.core").unwrap())
                .unwrap()
            {
                assert_eq!(paths.libraries, luasystem_expected_windows_libraries());
            };
        }
        if let Some(build::backend::Spec::Builtin(build::backend::Builtin { modules })) = &rockspec
            .local
            .build
            .get(&platform::Identifier::Win32)
            .backend
        {
            if let build::backend::ModuleSpec::ModulePaths(paths) = modules
                .get(&build::backend::Module::from_str("system.core").unwrap())
                .unwrap()
            {
                assert_eq!(paths.libraries, luasystem_expected_windows_libraries());
            };
        }
    }

    fn luasystem_expected_defines() -> Vec<(String, Option<String>)> {
        if cfg!(target_os = "windows") {
            vec![
                ("WINVER".into(), Some("0x0600".into())),
                ("_WIN32_WINNT".into(), Some("0x0600".into())),
            ]
        } else {
            Vec::default()
        }
    }

    fn luasystem_expected_windows_libraries() -> Vec<PathBuf> {
        vec!["advapi32".into(), "winmm".into()]
    }
    fn luasystem_expected_libraries() -> Vec<PathBuf> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            vec!["rt".into()]
        } else if cfg!(target_os = "windows") {
            luasystem_expected_windows_libraries()
        } else {
            Vec::default()
        }
    }

    fn luasystem_expected_incdirs() -> Vec<PathBuf> {
        Vec::default()
    }

    fn luasystem_expected_libdirs() -> Vec<PathBuf> {
        Vec::default()
    }

    #[test]
    pub fn rust_mlua_rockspec() {
        let rockspec_content = "
    package = 'foo'\n
    version = 'scm-1'\n
    source = {\n
        url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
    }\n
    build = {
        type = 'rust-mlua',
        modules = {
            'foo',
            bar = 'baz',
        },
        target_path = 'path/to/cargo/target/directory',
        default_features = false,
        include = {
            'file.lua',
            ['path/to/another/file.lua'] = 'another-file.lua',
        },
        features = {'extra', 'features'},
    }
            ";
        let rockspec = Remote::new(rockspec_content).unwrap();
        let build_spec = rockspec.local.build.current_platform();
        if let Some(build::backend::Spec::RustMlua(build_spec)) = build_spec.backend.to_owned() {
            assert_eq!(
                build_spec.modules.get("foo").unwrap(),
                &PathBuf::from(format!("libfoo.{}", std::env::consts::DLL_EXTENSION))
            );
            assert_eq!(
                build_spec.modules.get("bar").unwrap(),
                &PathBuf::from(format!("libbaz.{}", std::env::consts::DLL_EXTENSION))
            );
            assert_eq!(
                build_spec.include.get(&PathBuf::from("file.lua")).unwrap(),
                &PathBuf::from("file.lua")
            );
            assert_eq!(
                build_spec
                    .include
                    .get(&PathBuf::from("path/to/another/file.lua"))
                    .unwrap(),
                &PathBuf::from("another-file.lua")
            );
        } else {
            panic!("Expected RustMlua build backend");
        }
    }

    #[test]
    pub fn rust_binary_rockspec() {
        let rockspec_content = "
    package = 'foo'\n
    version = 'scm-1'\n
    source = {\n
        url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
    }\n
    build = {\n
        type = 'rust-binary',\n
        package = 'foo',\n
        features = {'extra', 'features'},\n
    }\n
            ";
        let rockspec = Remote::new(rockspec_content).unwrap();
        let build_spec = rockspec.local.build.current_platform();
        if let Some(build::backend::Spec::RustBinary(build_spec)) = build_spec.backend.to_owned() {
            assert_eq!(build_spec.package, Some("foo".to_string()));
            assert_eq!(build_spec.features, vec!["extra", "features"]);
        } else {
            panic!("Expected RustBinary build backend");
        }
    }

    #[tokio::test]
    pub async fn regression_ltui() {
        let content = String::from_utf8(
            std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/ltui-2.8-2.rockspec"),
            )
            .unwrap(),
        )
        .unwrap();
        Remote::new(&content).unwrap();
    }

    // Luarocks allows the `install.bin` field to be a list, even though it
    // should only allow a table.
    #[test]
    pub fn regression_off_spec_install_binaries() {
        let rockspec_content = r#"
            package = "WSAPI"
            version = "1.7-1"

            source = {
              url = "git://github.com/keplerproject/wsapi",
              tag = "v1.7",
            }

            build = {
              type = "builtin",
              modules = {
                ["wsapi"] = "src/wsapi.lua",
              },
              -- Offending Line
              install = { bin = { "src/launcher/wsapi.cgi" } }
            }
        "#;

        let rockspec = Remote::new(rockspec_content).unwrap();

        assert_eq!(
            rockspec.build().current_platform().install.bin,
            HashMap::from([("wsapi.cgi".into(), PathBuf::from("src/launcher/wsapi.cgi"))])
        );
    }

    #[test]
    pub fn regression_external_dependencies() {
        let content = String::from_utf8(
            std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/luaossl-20220711-0.rockspec"),
            )
            .unwrap(),
        )
        .unwrap();
        let rockspec = Remote::new(&content).unwrap();
        if cfg!(target_family = "unix") {
            assert_eq!(
                rockspec
                    .local
                    .external_dependencies
                    .current_platform()
                    .get("OPENSSL")
                    .unwrap(),
                &external_dependency::Spec {
                    library: Some("ssl".into()),
                    header: Some("openssl/ssl.h".into()),
                }
            );
        }
        let per_platform = rockspec.local.external_dependencies.per_platform;
        assert_eq!(
            *per_platform
                .get(&platform::Identifier::Windows)
                .and_then(|it| it.get("OPENSSL"))
                .unwrap(),
            external_dependency::Spec {
                library: Some("libeay32".into()),
                header: Some("openssl/ssl.h".into()),
            }
        );
    }

    #[test]
    pub fn remote_lua_rockspec_from_package_and_source_spec() {
        let package_req = "foo@1.0.5".parse().unwrap();
        let source = git::Source {
            url: "https://hub.com/owner/example-project.git".parse().unwrap(),
            git_ref: Some(git::Ref::Tag("1.0.5".into())),
        };
        let source_spec = source::Spec::Git(source);
        let rockspec =
            Remote::from_source_spec(package_req, source_spec.clone(), Vec::new(), HashMap::new());
        let generated_rockspec_str = rockspec.local.raw_content;
        let rockspec2 = Remote::new(&generated_rockspec_str).unwrap();
        assert_eq!(rockspec2.local.package, "foo".into());
        assert_eq!(rockspec2.local.version, "1.0.5".parse().unwrap());
        assert_eq!(rockspec2.local.source, PerPlatform::new(source_spec.into()));
    }

    #[test]
    pub fn regression_complex_source_field() {
        let rockspec_content = r#"
            package = "say"
            local rock_version = "1.4.1"
            local rock_release = "3"
            local namespace = "lunarmodules"
            local repository = package

            version = ("%s-%s"):format(rock_version, rock_release)

            source = {
              url = ("git+https://github.com/%s/%s.git"):format(namespace, repository),
              branch = rock_version == "scm" and "master" or nil,
              tag = rock_version ~= "scm" and "v"..rock_version or nil,
            }

            description = {
              summary = "Lua string hashing/indexing library",
            }

            dependencies = {
              "lua >= 5.1",
            }

            build = {
              type = "builtin",
            }
        "#
        .to_string();
        Remote::new(&rockspec_content).unwrap();
    }

    fn eval_lua_global<T: serde::de::DeserializeOwned>(code: &str, key: &'static str) -> T {
        use ottavino::{Closure, Executor, Fuel, Lua};
        use ottavino_util::serde::from_value;
        Lua::core()
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, None, code.as_bytes())?;
                let executor = Executor::start(ctx, closure.into(), ());
                executor.step(ctx, &mut Fuel::with(i32::MAX))?;
                from_value(ctx.globals().get_value(ctx, key)).map_err(ottavino::Error::from)
            })
            .unwrap()
    }

    #[test]
    pub fn rock_description_roundtrip() {
        let desc = description::Spec {
            summary: Some("A great package".into()),
            detailed: Some("Detailed description here".into()),
            license: Some("MIT".into()),
            homepage: Some("https://example.com".parse().unwrap()),
            issues_url: Some("https://example.com/issues".parse().unwrap()),
            maintainer: Some("Some Maintainer <maintainer@example.com>".into()),
            labels: vec!["neovim".into(), "lua".into()],
        };
        let lua = desc.display_lua().to_string();
        let restored: description::Spec = eval_lua_global(&lua, "description");
        assert_eq!(desc, restored);
    }

    #[test]
    pub fn rock_description_empty_roundtrip() {
        let desc = description::Spec::default();
        let lua = desc.display_lua().to_string();
        // An empty description produces an empty assignment; restore from empty table.
        let lua = if lua.trim().is_empty() {
            "description = {}".to_string()
        } else {
            lua
        };
        let restored: description::Spec = eval_lua_global(&lua, "description");
        assert_eq!(desc, restored);
    }

    #[test]
    pub fn regression_luasec() {
        let rockspec_content = r#"
package = "LuaSec"
version = "1.3.2-1"
source = {
  url = "git+https://github.com/brunoos/luasec",
  tag = "v1.3.2",
}
external_dependencies = {
   platforms = {
      unix = {
         OPENSSL = {
            header = "openssl/ssl.h",
            library = "ssl"
         }
      },
      windows = {
         OPENSSL = {
            header = "openssl/ssl.h",
         }
      },
   }
}
"#;
        let rockspec = Remote::new(rockspec_content).unwrap();
        let linux_external_deps = rockspec
            .external_dependencies()
            .get(&platform::Identifier::Linux);
        assert!(linux_external_deps.get("OPENSSL").is_some());
        let windows_external_deps = rockspec
            .external_dependencies()
            .get(&platform::Identifier::Windows);
        assert!(windows_external_deps.get("OPENSSL").is_some());
    }

    #[test]
    pub fn find_value_span_spaces() {
        let source = "package = 'foo'\nversion = '1.0'";
        let span = find_value_span(source, "package");
        assert_eq!(&source[span.offset()..span.offset() + span.len()], "'foo'");
    }

    #[test]
    pub fn find_value_span_no_spaces() {
        let source = "package='foo'";
        let span = find_value_span(source, "package");
        assert_eq!(&source[span.offset()..span.offset() + span.len()], "'foo'");
    }

    #[test]
    pub fn find_value_span_returns_empty_when_not_found() {
        let source = "version = '1.0'";
        let span = find_value_span(source, "package");
        assert_eq!(span.len(), 0);
    }

    #[test]
    pub fn find_value_span_skips_url_containing_field() {
        let source = "homepage = 'https://luapackager.com'\npackage = 'foo'";
        let span = find_value_span(source, "package");
        assert_eq!(&source[span.offset()..span.offset() + span.len()], "'foo'");
    }

    #[test]
    pub fn find_value_span_double_quoted() {
        let source = r#"package = "foo""#;
        let span = find_value_span(source, "package");
        assert_eq!(
            &source[span.offset()..span.offset() + span.len()],
            r#""foo""#
        );
    }

    #[test]
    pub fn find_value_span_table() {
        let source = "description = {\n    summary = 'bar'\n}";
        let span = find_value_span(source, "description");
        assert_eq!(&source[span.offset()..span.offset() + span.len()], "{");
    }

    #[test]
    pub fn parse_lua_rockspec_field_error() {
        let rockspec_content = "package = 'my-lua-lib'\nfoo = 123";
        let extern_err = ottavino::Lua::core()
            .try_enter(|_ctx| {
                Err(ParseLuaRockspecFieldError::Deserialize {
                    field: "foo".into(),
                    source: Box::new(ottavino_util::serde::de::Error::Message(
                        "expected bar".into(),
                    )),
                })?;
                Ok(())
            })
            .unwrap_err();
        let err = Error::from_ottavino(extern_err, rockspec_content);
        match err {
            Error::ExecutionError {
                cause,
                err_span: Some(span),
                ..
            } => {
                assert!(cause.contains("invalid rockspec field 'foo'"));
                assert_eq!(
                    &rockspec_content[span.offset()..span.offset() + span.len()],
                    "foo"
                );
            }
            other => panic!("expected ExecutionError with err_span, got: {other:?}"),
        }
        let extern_err = ottavino::Lua::core()
            .try_enter(|_ctx| {
                Err(ParseLuaRockspecFieldError::DeserializeTable {
                    field: "foo".into(),
                    source: Box::new(ottavino_util::serde::de::Error::Message(
                        "expected bar".into(),
                    )),
                })?;
                Ok(())
            })
            .unwrap_err();
        let err = Error::from_ottavino(extern_err, rockspec_content);
        match err {
            Error::ExecutionError {
                cause,
                err_span: Some(span),
                ..
            } => {
                assert!(cause.contains("invalid rockspec field 'foo'"));
                assert_eq!(
                    &rockspec_content[span.offset()..span.offset() + span.len()],
                    "foo"
                );
            }
            other => panic!("expected ExecutionError with err_span, got: {other:?}"),
        }
        let extern_err = ottavino::Lua::core()
            .try_enter(|_ctx| {
                Err(ParseLuaRockspecFieldError::NotATable {
                    field: "foo".into(),
                    invalid_type: "integer".into(),
                })?;
                Ok(())
            })
            .unwrap_err();
        let err = Error::from_ottavino(extern_err, rockspec_content);
        match err {
            Error::ExecutionError {
                cause,
                err_span: Some(span),
                ..
            } => {
                assert!(cause.contains("invalid rockspec field 'foo'"));
                assert_eq!(
                    &rockspec_content[span.offset()..span.offset() + span.len()],
                    "foo"
                );
            }
            other => panic!("expected ExecutionError with err_span, got: {other:?}"),
        }
    }
}
