mod build;
mod dependency;
mod deploy;
mod partial;
mod platform;
mod rock_source;
mod serde_util;
mod test_spec;

use std::{
    collections::HashMap, convert::Infallible, fmt::Display, io, path::PathBuf, str::FromStr,
};

use mlua::{FromLua, IntoLua, Lua, LuaSerdeExt, UserData, Value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub use build::*;
pub use dependency::*;
pub use deploy::*;
pub use partial::*;
pub use platform::*;
pub use rock_source::*;
use ssri::Integrity;
pub use test_spec::*;
use thiserror::Error;
use url::Url;

pub(crate) use serde_util::*;

use crate::{
    config::{LuaVersion, LuaVersionUnset},
    hash::HasIntegrity,
    package::{PackageName, PackageSpec, PackageVersion, PackageVersionReq},
    project::project_toml::ProjectTomlError,
    project::ProjectRoot,
    rockspec::{lua_dependency::LuaDependencySpec, Rockspec},
};

#[derive(Error, Debug)]
pub enum LuaRockspecError {
    #[error("could not parse rockspec: {0}")]
    MLua(#[from] mlua::Error),
    #[error("{}copy_directories cannot contain the rockspec name", ._0.as_ref().map(|p| format!("{p}: ")).unwrap_or_default())]
    CopyDirectoriesContainRockspecName(Option<String>),
    #[error("could not parse rockspec: {0}")]
    LuaTable(#[from] LuaTableError),
    #[error("cannot create Lua rockspec with off-spec dependency: {0}")]
    OffSpecDependency(PackageName),
    #[error("cannot create Lua rockspec with off-spec build dependency: {0}")]
    OffSpecBuildDependency(PackageName),
    #[error("cannot create Lua rockspec with off-spec test dependency: {0}")]
    OffSpecTestDependency(PackageName),
    #[error(transparent)]
    ProjectToml(#[from] ProjectTomlError),
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct LocalLuaRockspec {
    /// The file format version. Example: "1.0"
    rockspec_format: Option<RockspecFormat>,
    /// The name of the package. Example: "luasocket"
    package: PackageName,
    /// The version of the package, plus a suffix indicating the revision of the rockspec. Example: "2.0.1-1"
    version: PackageVersion,
    description: RockDescription,
    supported_platforms: PlatformSupport,
    /// The Lua version requirement for this rock
    lua: PackageVersionReq,
    dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    build_dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    external_dependencies: PerPlatform<HashMap<String, ExternalDependencySpec>>,
    test_dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    build: PerPlatform<BuildSpec>,
    source: PerPlatform<RemoteRockSource>,
    test: PerPlatform<TestSpec>,
    deploy: PerPlatform<DeploySpec>,
    /// The original content of this rockspec, needed by luarocks
    raw_content: String,
}

impl UserData for LocalLuaRockspec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, _: ()| Ok(this.package.clone()));
        methods.add_method("version", |_, this, _: ()| Ok(this.version.clone()));
        methods.add_method("description", |_, this, _: ()| Ok(this.description.clone()));
        methods.add_method("supported_platforms", |_, this, _: ()| {
            Ok(this.supported_platforms.clone())
        });
        methods.add_method("lua", |_, this, _: ()| Ok(this.lua.clone()));
        methods.add_method("dependencies", |_, this, _: ()| {
            Ok(this.dependencies.clone())
        });
        methods.add_method("build_dependencies", |_, this, _: ()| {
            Ok(this.build_dependencies.clone())
        });
        methods.add_method("external_dependencies", |_, this, _: ()| {
            Ok(this.external_dependencies.clone())
        });
        methods.add_method("test_dependencies", |_, this, _: ()| {
            Ok(this.test_dependencies.clone())
        });
        methods.add_method("build", |_, this, _: ()| Ok(this.build.clone()));
        methods.add_method("source", |_, this, _: ()| Ok(this.source.clone()));
        methods.add_method("test", |_, this, _: ()| Ok(this.test.clone()));
        methods.add_method("format", |_, this, _: ()| Ok(this.rockspec_format.clone()));

        methods.add_method("to_lua_rockspec_string", |_, this, _: ()| {
            this.to_lua_remote_rockspec_string()
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
    }
}

impl LocalLuaRockspec {
    pub fn new(
        rockspec_content: &str,
        project_root: ProjectRoot,
    ) -> Result<Self, LuaRockspecError> {
        let lua = Lua::new();
        lua.load(rockspec_content).exec()?;

        let globals = lua.globals();

        let dependencies: PerPlatform<Vec<LuaDependencySpec>> = globals.get("dependencies")?;

        let lua_version_req = dependencies
            .current_platform()
            .iter()
            .find(|dep| dep.name().to_string() == "lua")
            .cloned()
            .map(|dep| dep.version_req().clone())
            .unwrap_or(PackageVersionReq::Any);

        fn strip_lua(
            dependencies: PerPlatform<Vec<LuaDependencySpec>>,
        ) -> PerPlatform<Vec<LuaDependencySpec>> {
            dependencies.map(|deps| {
                deps.iter()
                    .filter(|dep| dep.name().to_string() != "lua")
                    .cloned()
                    .collect()
            })
        }

        let build_dependencies: PerPlatform<Vec<LuaDependencySpec>> =
            globals.get("build_dependencies")?;

        let test_dependencies: PerPlatform<Vec<LuaDependencySpec>> =
            globals.get("test_dependencies")?;

        let rockspec = LocalLuaRockspec {
            rockspec_format: globals.get("rockspec_format")?,
            package: globals.get("package")?,
            version: globals.get("version")?,
            description: parse_lua_tbl_or_default(&lua, "description")?,
            supported_platforms: parse_lua_tbl_or_default(&lua, "supported_platforms")?,
            lua: lua_version_req,
            dependencies: strip_lua(dependencies),
            build_dependencies: strip_lua(build_dependencies),
            test_dependencies: strip_lua(test_dependencies),
            external_dependencies: globals.get("external_dependencies")?,
            build: globals.get("build")?,
            test: globals.get("test")?,
            deploy: globals.get("deploy")?,
            raw_content: rockspec_content.into(),

            source: globals
                .get::<Option<PerPlatform<RemoteRockSource>>>("source")?
                .unwrap_or_else(|| {
                    PerPlatform::new(RockSourceSpec::File(project_root.to_path_buf()).into())
                }),
        };

        let rockspec_file_name = format!("{}-{}.rockspec", rockspec.package(), rockspec.version());
        if rockspec
            .build()
            .default
            .copy_directories
            .contains(&PathBuf::from(&rockspec_file_name))
        {
            return Err(LuaRockspecError::CopyDirectoriesContainRockspecName(None));
        }

        for (platform, build_override) in &rockspec.build().per_platform {
            if build_override
                .copy_directories
                .contains(&PathBuf::from(&rockspec_file_name))
            {
                return Err(LuaRockspecError::CopyDirectoriesContainRockspecName(Some(
                    platform.to_string(),
                )));
            }
        }
        Ok(rockspec)
    }
}

impl Rockspec for LocalLuaRockspec {
    type Error = Infallible;

    fn package(&self) -> &PackageName {
        &self.package
    }

    fn version(&self) -> &PackageVersion {
        &self.version
    }

    fn description(&self) -> &RockDescription {
        &self.description
    }

    fn supported_platforms(&self) -> &PlatformSupport {
        &self.supported_platforms
    }

    fn lua(&self) -> &PackageVersionReq {
        &self.lua
    }

    fn dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        &self.dependencies
    }

    fn build_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        &self.build_dependencies
    }

    fn external_dependencies(&self) -> &PerPlatform<HashMap<String, ExternalDependencySpec>> {
        &self.external_dependencies
    }

    fn test_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        &self.test_dependencies
    }

    fn build(&self) -> &PerPlatform<BuildSpec> {
        &self.build
    }

    fn test(&self) -> &PerPlatform<TestSpec> {
        &self.test
    }

    fn source(&self) -> &PerPlatform<RemoteRockSource> {
        &self.source
    }

    fn deploy(&self) -> &PerPlatform<DeploySpec> {
        &self.deploy
    }

    fn build_mut(&mut self) -> &mut PerPlatform<BuildSpec> {
        &mut self.build
    }

    fn test_mut(&mut self) -> &mut PerPlatform<TestSpec> {
        &mut self.test
    }

    fn source_mut(&mut self) -> &mut PerPlatform<RemoteRockSource> {
        &mut self.source
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<DeploySpec> {
        &mut self.deploy
    }

    fn format(&self) -> &Option<RockspecFormat> {
        &self.rockspec_format
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, Self::Error> {
        Ok(self.raw_content.clone())
    }
}

impl HasIntegrity for LocalLuaRockspec {
    fn hash(&self) -> io::Result<Integrity> {
        Ok(Integrity::from(&self.raw_content))
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct RemoteLuaRockspec {
    local: LocalLuaRockspec,
    source: PerPlatform<RemoteRockSource>,
}

impl UserData for RemoteLuaRockspec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, _: ()| Ok(this.local.package.clone()));
        methods.add_method("version", |_, this, _: ()| Ok(this.local.version.clone()));
        methods.add_method("description", |_, this, _: ()| {
            Ok(this.local.description.clone())
        });
        methods.add_method("supported_platforms", |_, this, _: ()| {
            Ok(this.local.supported_platforms.clone())
        });
        methods.add_method("lua", |_, this, _: ()| Ok(this.local.lua.clone()));
        methods.add_method("dependencies", |_, this, _: ()| {
            Ok(this.local.dependencies.clone())
        });
        methods.add_method("build_dependencies", |_, this, _: ()| {
            Ok(this.local.build_dependencies.clone())
        });
        methods.add_method("external_dependencies", |_, this, _: ()| {
            Ok(this.local.external_dependencies.clone())
        });
        methods.add_method("test_dependencies", |_, this, _: ()| {
            Ok(this.local.test_dependencies.clone())
        });
        methods.add_method("build", |_, this, _: ()| Ok(this.local.build.clone()));
        methods.add_method("source", |_, this, _: ()| Ok(this.source.clone()));
        methods.add_method("test", |_, this, _: ()| Ok(this.local.test.clone()));
        methods.add_method("format", |_, this, _: ()| {
            Ok(this.local.rockspec_format.clone())
        });

        methods.add_method("to_lua_rockspec_string", |_, this, _: ()| {
            this.to_lua_remote_rockspec_string()
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
    }
}

impl RemoteLuaRockspec {
    pub fn new(rockspec_content: &str) -> Result<Self, LuaRockspecError> {
        let lua = Lua::new();
        lua.load(rockspec_content).exec()?;

        let globals = lua.globals();
        let source = globals.get("source")?;

        let rockspec = RemoteLuaRockspec {
            local: LocalLuaRockspec::new(rockspec_content, ProjectRoot::new())?,
            source,
        };

        Ok(rockspec)
    }

    pub fn from_package_and_source_spec(
        package_spec: PackageSpec,
        source_spec: RockSourceSpec,
    ) -> Self {
        let version = package_spec.version().clone();
        let rockspec_format = RockspecFormat::default();
        let raw_content = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}"
{}
build = {{
  type = "source"
}}"#,
            &rockspec_format,
            package_spec.name(),
            &version,
            &source_spec.display_lua(),
        );

        let source: RemoteRockSource = source_spec.into();

        let local = LocalLuaRockspec {
            rockspec_format: Some(rockspec_format),
            package: package_spec.name().clone(),
            version,
            description: RockDescription::default(),
            supported_platforms: PlatformSupport::default(),
            lua: PackageVersionReq::Any,
            dependencies: PerPlatform::default(),
            build_dependencies: PerPlatform::default(),
            external_dependencies: PerPlatform::default(),
            test_dependencies: PerPlatform::default(),
            build: PerPlatform::new(BuildSpec {
                build_backend: Some(BuildBackendSpec::Source),
                install: InstallSpec::default(),
                copy_directories: Vec::new(),
                patches: HashMap::new(),
            }),
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

impl Rockspec for RemoteLuaRockspec {
    type Error = Infallible;

    fn package(&self) -> &PackageName {
        self.local.package()
    }

    fn version(&self) -> &PackageVersion {
        self.local.version()
    }

    fn description(&self) -> &RockDescription {
        self.local.description()
    }

    fn supported_platforms(&self) -> &PlatformSupport {
        self.local.supported_platforms()
    }

    fn lua(&self) -> &PackageVersionReq {
        self.local.lua()
    }

    fn dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        self.local.dependencies()
    }

    fn build_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        match self.format() {
            // Rockspec formats < 3.0 don't support `build_dependencies`,
            // so we have to return regular dependencies if the build backend might need to use them.
            Some(RockspecFormat::_1_0 | RockspecFormat::_2_0)
                if self
                    .build()
                    .current_platform()
                    .build_backend
                    .as_ref()
                    .is_some_and(|build_backend| build_backend.can_use_build_dependencies()) =>
            {
                self.local.dependencies()
            }
            _ => self.local.build_dependencies(),
        }
    }

    fn external_dependencies(&self) -> &PerPlatform<HashMap<String, ExternalDependencySpec>> {
        self.local.external_dependencies()
    }

    fn test_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>> {
        self.local.test_dependencies()
    }

    fn build(&self) -> &PerPlatform<BuildSpec> {
        self.local.build()
    }

    fn test(&self) -> &PerPlatform<TestSpec> {
        self.local.test()
    }

    fn source(&self) -> &PerPlatform<RemoteRockSource> {
        &self.source
    }

    fn deploy(&self) -> &PerPlatform<DeploySpec> {
        self.local.deploy()
    }

    fn build_mut(&mut self) -> &mut PerPlatform<BuildSpec> {
        self.local.build_mut()
    }

    fn test_mut(&mut self) -> &mut PerPlatform<TestSpec> {
        self.local.test_mut()
    }

    fn source_mut(&mut self) -> &mut PerPlatform<RemoteRockSource> {
        &mut self.source
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<DeploySpec> {
        self.local.deploy_mut()
    }

    fn format(&self) -> &Option<RockspecFormat> {
        self.local.format()
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, Self::Error> {
        Ok(self.local.raw_content.clone())
    }
}

#[derive(Error, Debug)]
pub enum LuaVersionError {
    #[error("The lua version {0} is not supported by {1} version {2}!")]
    LuaVersionUnsupported(LuaVersion, PackageName, PackageVersion),
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
}

impl HasIntegrity for RemoteLuaRockspec {
    fn hash(&self) -> io::Result<Integrity> {
        Ok(Integrity::from(&self.local.raw_content))
    }
}

#[derive(Clone, Deserialize, Debug, PartialEq, Default)]
pub struct RockDescription {
    /// A one-line description of the package.
    pub summary: Option<String>,
    /// A longer description of the package.
    pub detailed: Option<String>,
    /// The license used by the package.
    pub license: Option<String>,
    /// An URL for the project. This is not the URL for the tarball, but the address of a website.
    #[serde(default, deserialize_with = "deserialize_url")]
    pub homepage: Option<Url>,
    /// An URL for the project's issue tracker.
    pub issues_url: Option<String>,
    /// Contact information for the rockspec maintainer.
    pub maintainer: Option<String>,
    /// A list of short strings that specify labels for categorization of this rock.
    #[serde(default)]
    pub labels: Vec<String>,
}

fn deserialize_url<'de, D>(deserializer: D) -> Result<Option<Url>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    s.map(|s| Url::parse(&s).map_err(serde::de::Error::custom))
        .transpose()
}

impl DisplayAsLuaKV for RockDescription {
    fn display_lua(&self) -> DisplayLuaKV {
        let mut description = Vec::new();

        if let Some(summary) = &self.summary {
            description.push(DisplayLuaKV {
                key: "summary".to_string(),
                value: DisplayLuaValue::String(summary.clone()),
            })
        }
        if let Some(detailed) = &self.detailed {
            description.push(DisplayLuaKV {
                key: "detailed".to_string(),
                value: DisplayLuaValue::String(detailed.clone()),
            })
        }
        if let Some(license) = &self.license {
            description.push(DisplayLuaKV {
                key: "license".to_string(),
                value: DisplayLuaValue::String(license.clone()),
            })
        }
        if let Some(homepage) = &self.homepage {
            description.push(DisplayLuaKV {
                key: "homepage".to_string(),
                value: DisplayLuaValue::String(homepage.to_string()),
            })
        }
        if let Some(issues_url) = &self.issues_url {
            description.push(DisplayLuaKV {
                key: "issues_url".to_string(),
                value: DisplayLuaValue::String(issues_url.clone()),
            })
        }
        if let Some(maintainer) = &self.maintainer {
            description.push(DisplayLuaKV {
                key: "maintainer".to_string(),
                value: DisplayLuaValue::String(maintainer.clone()),
            })
        }
        if !self.labels.is_empty() {
            description.push(DisplayLuaKV {
                key: "labels".to_string(),
                value: DisplayLuaValue::List(
                    self.labels
                        .iter()
                        .cloned()
                        .map(DisplayLuaValue::String)
                        .collect(),
                ),
            })
        }

        DisplayLuaKV {
            key: "description".to_string(),
            value: DisplayLuaValue::Table(description),
        }
    }
}

impl UserData for RockDescription {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("summary", |_, this, _: ()| Ok(this.summary.clone()));
        methods.add_method("detailed", |_, this, _: ()| Ok(this.detailed.clone()));
        methods.add_method("license", |_, this, _: ()| Ok(this.license.clone()));
        methods.add_method("homepage", |_, this, _: ()| {
            Ok(this.homepage.clone().map(|url| url.to_string()))
        });
        methods.add_method("issues_url", |_, this, _: ()| Ok(this.issues_url.clone()));
        methods.add_method("maintainer", |_, this, _: ()| Ok(this.maintainer.clone()));
        methods.add_method("labels", |_, this, _: ()| Ok(this.labels.clone()));
    }
}

#[derive(Error, Debug)]
#[error("invalid rockspec format: {0}")]
pub struct InvalidRockspecFormat(String);

#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RockspecFormat {
    #[serde(rename = "1.0")]
    _1_0,
    #[serde(rename = "2.0")]
    _2_0,
    #[serde(rename = "3.0")]
    #[default]
    _3_0,
}

impl FromStr for RockspecFormat {
    type Err = InvalidRockspecFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1.0" => Ok(Self::_1_0),
            "2.0" => Ok(Self::_2_0),
            "3.0" => Ok(Self::_3_0),
            txt => Err(InvalidRockspecFormat(txt.to_string())),
        }
    }
}

impl FromLua for RockspecFormat {
    fn from_lua(
        value: mlua::prelude::LuaValue,
        lua: &mlua::prelude::Lua,
    ) -> mlua::prelude::LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        Self::from_str(&s).map_err(|err| mlua::Error::DeserializeError(err.to_string()))
    }
}

impl IntoLua for RockspecFormat {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        self.to_string().into_lua(lua)
    }
}

impl Display for RockspecFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::_1_0 => write!(f, "1.0"),
            Self::_2_0 => write!(f, "2.0"),
            Self::_3_0 => write!(f, "3.0"),
        }
    }
}

#[derive(Error, Debug)]
pub enum LuaTableError {
    #[error("could not parse {variable}. Expected list, but got {invalid_type}")]
    ParseError {
        variable: String,
        invalid_type: String,
    },
    #[error(transparent)]
    MLua(#[from] mlua::Error),
}

fn parse_lua_tbl_or_default<T>(lua: &Lua, lua_var_name: &str) -> Result<T, LuaTableError>
where
    T: Default,
    T: DeserializeOwned,
{
    let ret = match lua.globals().get(lua_var_name)? {
        Value::Nil => T::default(),
        value @ Value::Table(_) => lua.from_value(value)?,
        value => Err(LuaTableError::ParseError {
            variable: lua_var_name.to_string(),
            invalid_type: value.type_name().to_string(),
        })?,
    };
    Ok(ret)
}

#[cfg(test)]
mod tests {

    use std::path::PathBuf;

    use crate::git::GitSource;
    use crate::lua_rockspec::PlatformIdentifier;
    use crate::package::PackageSpec;

    use super::*;

    #[tokio::test]
    pub async fn parse_rockspec() {
        let rockspec_content = "
        rockspec_format = '1.0'\n
        package = 'foo'\n
        version = '1.0.0-1'\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.rockspec_format, Some(RockspecFormat::_1_0));
        assert_eq!(rockspec.local.package, "foo".into());
        assert_eq!(rockspec.local.version, "1.0.0-1".parse().unwrap());
        assert_eq!(rockspec.local.description, RockDescription::default());

        let rockspec_content = "
        package = 'bar'\n
        version = '2.0.0-1'\n
        description = {}\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.rockspec_format, None);
        assert_eq!(rockspec.local.package, "bar".into());
        assert_eq!(rockspec.local.version, "2.0.0-1".parse().unwrap());
        assert_eq!(rockspec.local.description, RockDescription::default());

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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.rockspec_format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = RockDescription {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some("https://github.com/lumen-oss/rocks.nvim/issues".into()),
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.rockspec_format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = RockDescription {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some("https://github.com/lumen-oss/rocks.nvim/issues".into()),
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
            ExternalDependencySpec {
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.rockspec_format, None);
        assert_eq!(rockspec.local.package, "rocks.nvim".into());
        assert_eq!(rockspec.local.version, "3.0.0-1".parse().unwrap());
        let expected_description = RockDescription {
            summary: Some("some summary".into()),
            detailed: Some("some detailed description".into()),
            license: Some("MIT".into()),
            homepage: Some(Url::parse("https://github.com/lumen-oss/rocks.nvim").unwrap()),
            issues_url: Some("https://github.com/lumen-oss/rocks.nvim/issues".into()),
            maintainer: Some("Lumen Labs".into()),
            labels: vec!["package management".into()],
        };
        assert_eq!(rockspec.local.description, expected_description);
        assert!(rockspec
            .local
            .supported_platforms
            .is_supported(&PlatformIdentifier::Unix));
        assert!(!rockspec
            .local
            .supported_platforms
            .is_supported(&PlatformIdentifier::Windows));
        let neorg = PackageSpec::parse("neorg".into(), "6.0.0".into()).unwrap();
        assert!(rockspec
            .local
            .dependencies
            .default
            .into_iter()
            .any(|dep| dep.matches(&neorg)));
        let foo = PackageSpec::parse("foo".into(), "1.0.0".into()).unwrap();
        assert!(rockspec
            .local
            .build_dependencies
            .default
            .into_iter()
            .any(|dep| dep.matches(&foo)));
        let busted = PackageSpec::parse("busted".into(), "2.2.0".into()).unwrap();
        assert_eq!(
            *rockspec
                .local
                .external_dependencies
                .default
                .get("FOO")
                .unwrap(),
            ExternalDependencySpec {
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            RockSourceSpec::Git(GitSource {
                url: "https://hub.com/owner/example-project/".parse().unwrap(),
                checkout_ref: Some("bar".into())
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            RockSourceSpec::Git(GitSource {
                url: "https://hub.com/owner/example-project/".parse().unwrap(),
                checkout_ref: Some("bar".into())
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
        let _rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap_err();
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert!(matches!(
            rockspec.local.build.default.build_backend,
            Some(BuildBackendSpec::Builtin { .. })
        ));
        let install_lua_spec = rockspec.local.build.default.install.lua;
        let foo_bar_path = install_lua_spec
            .get(&LuaModule::from_str("foo.bar").unwrap())
            .unwrap();
        assert_eq!(*foo_bar_path, PathBuf::from("src/bar.lua"));
        let foo_path = install_lua_spec
            .get(&LuaModule::from_str("foo").unwrap())
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
        let _rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap_err();
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
        let _rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap_err();
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
        let _rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap_err();
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.source.default.unpack_dir, Some("baz".into()));
        assert_eq!(
            rockspec.local.build.default.build_backend,
            Some(BuildBackendSpec::Make(MakeBuildSpec::default()))
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.build.default.build_backend,
            Some(BuildBackendSpec::CMake(CMakeBuildSpec::default()))
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert!(matches!(
            rockspec.local.build.default.build_backend,
            Some(BuildBackendSpec::Command(CommandBuildSpec { .. }))
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
        RemoteLuaRockspec::new(&rockspec_content).unwrap();
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
        RemoteLuaRockspec::new(&rockspec_content).unwrap();
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        let neorg_override = PackageSpec::parse("neorg".into(), "5.0.0".into()).unwrap();
        let toml_edit = PackageSpec::parse("toml-edit".into(), "1.0.0".into()).unwrap();
        let toml = PackageSpec::parse("toml".into(), "1.0.0".into()).unwrap();
        assert_eq!(rockspec.local.dependencies.default.len(), 2);
        let per_platform = &rockspec.local.dependencies.per_platform;
        assert_eq!(
            per_platform
                .get(&PlatformIdentifier::Windows)
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
                .get(&PlatformIdentifier::Unix)
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
                .get(&PlatformIdentifier::Linux)
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(
            *rockspec
                .local
                .external_dependencies
                .default
                .get("FOO")
                .unwrap(),
            ExternalDependencySpec {
                library: Some("foo".into()),
                header: None
            }
        );
        let per_platform = rockspec.local.external_dependencies.per_platform;
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Windows)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            ExternalDependencySpec {
                library: Some("foo.dll".into()),
                header: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Unix)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            ExternalDependencySpec {
                library: Some("foo".into()),
                header: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Unix)
                .and_then(|it| it.get("BAR"))
                .unwrap(),
            ExternalDependencySpec {
                header: Some("bar.h".into()),
                library: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Linux)
                .and_then(|it| it.get("BAR"))
                .unwrap(),
            ExternalDependencySpec {
                header: Some("bar.h".into()),
                library: None
            }
        );
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Linux)
                .and_then(|it| it.get("FOO"))
                .unwrap(),
            ExternalDependencySpec {
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(
            rockspec.local.source.default.source_spec,
            RockSourceSpec::Git(GitSource {
                url: "https://hub.com/example-project/.git".parse().unwrap(),
                checkout_ref: Some("bar".into())
            })
        );
        assert_eq!(
            rockspec
                .source
                .per_platform
                .get(&PlatformIdentifier::MacOSX)
                .map(|it| it.source_spec.clone())
                .unwrap(),
            RockSourceSpec::Git(GitSource {
                url: "https://hub.com/example-project/.git".parse().unwrap(),
                checkout_ref: Some("mac".into())
            })
        );
        assert_eq!(
            rockspec
                .source
                .per_platform
                .get(&PlatformIdentifier::Windows)
                .map(|it| it.source_spec.clone())
                .unwrap(),
            RockSourceSpec::Git(GitSource {
                url: "https://winhub.com/example-project/.git".parse().unwrap(),
                checkout_ref: Some("win".into())
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        let per_platform = rockspec.local.build.per_platform;
        let unix = per_platform.get(&PlatformIdentifier::Unix).unwrap();
        assert_eq!(
            unix.copy_directories,
            vec![PathBuf::from("plugin"), PathBuf::from("ftplugin")]
        );
        let linux = per_platform.get(&PlatformIdentifier::Linux).unwrap();
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
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        let per_platform = &rockspec.local.build.per_platform;
        let win32 = per_platform.get(&PlatformIdentifier::Windows).unwrap();
        assert_eq!(
            win32.build_backend,
            Some(BuildBackendSpec::Builtin(BuiltinBuildSpec {
                modules: vec![(
                    LuaModule::from_str("cjson").unwrap(),
                    ModuleSpec::ModulePaths(ModulePaths {
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
        let rockspec = RemoteLuaRockspec::new(rockspec_content).unwrap();
        let deploy_spec = &rockspec.deploy().current_platform();
        assert!(!deploy_spec.wrap_bin_scripts);
    }

    #[tokio::test]
    pub async fn parse_scm_rockspec() {
        let rockspec_content = "
        package = 'foo'\n
        version = 'scm-1'\n
        source = {\n
            url = 'https://github.com/lumen-oss/rocks.nvim/archive/1.0.0/rocks.nvim.zip',\n
        }\n
        "
        .to_string();
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        assert_eq!(rockspec.local.package, "foo".into());
        assert_eq!(rockspec.local.version, "scm-1".parse().unwrap());
    }

    #[tokio::test]
    pub async fn regression_luasystem() {
        let rockspec_content =
            String::from_utf8(std::fs::read("resources/test/luasystem-0.4.4-1.rockspec").unwrap())
                .unwrap();
        let rockspec = RemoteLuaRockspec::new(&rockspec_content).unwrap();
        let build_spec = rockspec.local.build.current_platform();
        assert!(matches!(
            build_spec.build_backend,
            Some(BuildBackendSpec::Builtin { .. })
        ));
        if let Some(BuildBackendSpec::Builtin(BuiltinBuildSpec { modules })) =
            &build_spec.build_backend
        {
            assert_eq!(
                modules.get(&LuaModule::from_str("system.init").unwrap()),
                Some(&ModuleSpec::SourcePath("system/init.lua".into()))
            );
            assert_eq!(
                modules.get(&LuaModule::from_str("system.core").unwrap()),
                Some(&ModuleSpec::ModulePaths(ModulePaths {
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
                }))
            );
        }
        if let Some(BuildBackendSpec::Builtin(BuiltinBuildSpec { modules })) = &rockspec
            .local
            .build
            .get(&PlatformIdentifier::Windows)
            .build_backend
        {
            if let ModuleSpec::ModulePaths(paths) = modules
                .get(&LuaModule::from_str("system.core").unwrap())
                .unwrap()
            {
                assert_eq!(paths.libraries, luasystem_expected_windows_libraries());
            };
        }
        if let Some(BuildBackendSpec::Builtin(BuiltinBuildSpec { modules })) = &rockspec
            .local
            .build
            .get(&PlatformIdentifier::Win32)
            .build_backend
        {
            if let ModuleSpec::ModulePaths(paths) = modules
                .get(&LuaModule::from_str("system.core").unwrap())
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
        if cfg!(target_os = "linux") {
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

    #[tokio::test]
    pub async fn rust_mlua_rockspec() {
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
        let rockspec = RemoteLuaRockspec::new(rockspec_content).unwrap();
        let build_spec = rockspec.local.build.current_platform();
        if let Some(BuildBackendSpec::RustMlua(build_spec)) = build_spec.build_backend.to_owned() {
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

    #[tokio::test]
    pub async fn regression_ltui() {
        let content =
            String::from_utf8(std::fs::read("resources/test/ltui-2.8-2.rockspec").unwrap())
                .unwrap();
        RemoteLuaRockspec::new(&content).unwrap();
    }

    // Luarocks allows the `install.bin` field to be a list, even though it
    // should only allow a table.
    #[tokio::test]
    pub async fn regression_off_spec_install_binaries() {
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

        let rockspec = RemoteLuaRockspec::new(rockspec_content).unwrap();

        assert_eq!(
            rockspec.build().current_platform().install.bin,
            HashMap::from([("wsapi.cgi".into(), PathBuf::from("src/launcher/wsapi.cgi"))])
        );
    }

    #[tokio::test]
    pub async fn regression_external_dependencies() {
        let content =
            String::from_utf8(std::fs::read("resources/test/luaossl-20220711-0.rockspec").unwrap())
                .unwrap();
        let rockspec = RemoteLuaRockspec::new(&content).unwrap();
        if cfg!(target_family = "unix") {
            assert_eq!(
                rockspec
                    .local
                    .external_dependencies
                    .current_platform()
                    .get("OPENSSL")
                    .unwrap(),
                &ExternalDependencySpec {
                    library: Some("ssl".into()),
                    header: Some("openssl/ssl.h".into()),
                }
            );
        }
        let per_platform = rockspec.local.external_dependencies.per_platform;
        assert_eq!(
            *per_platform
                .get(&PlatformIdentifier::Windows)
                .and_then(|it| it.get("OPENSSL"))
                .unwrap(),
            ExternalDependencySpec {
                library: Some("libeay32".into()),
                header: Some("openssl/ssl.h".into()),
            }
        );
    }

    #[tokio::test]
    pub async fn remote_lua_rockspec_from_package_and_source_spec() {
        let package_req = "foo@1.0.5".parse().unwrap();
        let source = GitSource {
            url: "https://hub.com/owner/example-project.git".parse().unwrap(),
            checkout_ref: Some("1.0.5".into()),
        };
        let source_spec = RockSourceSpec::Git(source);
        let rockspec =
            RemoteLuaRockspec::from_package_and_source_spec(package_req, source_spec.clone());
        let generated_rockspec_str = rockspec.local.raw_content;
        let rockspec2 = RemoteLuaRockspec::new(&generated_rockspec_str).unwrap();
        assert_eq!(rockspec2.local.package, "foo".into());
        assert_eq!(rockspec2.local.version, "1.0.5".parse().unwrap());
        assert_eq!(rockspec2.local.source, PerPlatform::new(source_spec.into()));
    }
}
