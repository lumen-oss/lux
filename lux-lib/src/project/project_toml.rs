//! Structs and utilities for `lux.toml`

use crate::git::shorthand::RemoteGitUrlShorthand;
use crate::git::GitSource;
use crate::hash::HasIntegrity;
use crate::lockfile::OptState;
use crate::lockfile::PinnedState;
use crate::lua_rockspec::DeploySpec;
use crate::lua_rockspec::LocalLuaRockspec;
use crate::lua_rockspec::LocalRockSource;
use crate::lua_rockspec::LuaRockspecError;
use crate::lua_rockspec::RemoteLuaRockspec;
use crate::lua_rockspec::RockSourceSpec;
use crate::operations::RunCommand;
use crate::package::PackageNameList;
use crate::package::SpecRev;
use crate::rockspec::lua_dependency::LuaDependencySpec;
use std::io;
use std::{collections::HashMap, path::PathBuf};

use itertools::Itertools;
use mlua::ExternalResult;
use mlua::UserData;
use nonempty::NonEmpty;
use serde::de;
use serde::{Deserialize, Deserializer};
use ssri::Integrity;
use thiserror::Error;

use crate::{
    config::{Config, LuaVersion},
    lua_rockspec::{
        BuildSpec, BuildSpecInternal, BuildSpecInternalError, DisplayAsLuaKV, ExternalDependencies,
        ExternalDependencySpec, LuaVersionError, PartialLuaRockspec, PerPlatform,
        PlatformIdentifier, PlatformSupport, PlatformValidationError, RemoteRockSource,
        RockDescription, RockSourceError, RockspecFormat, TestSpec, TestSpecDecodeError,
        TestSpecInternal,
    },
    package::{
        BuildDependencies, Dependencies, PackageName, PackageReq, PackageVersion, PackageVersionReq,
    },
    rockspec::{LuaVersionCompatibility, Rockspec},
};

use super::gen::GenerateSourceError;
use super::gen::RockSourceTemplate;
use super::r#gen::GenerateVersionError;
use super::r#gen::PackageVersionTemplate;
use super::ProjectRoot;

pub const PROJECT_TOML: &str = "lux.toml";

#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // This is ok because it's just a Deserialize helper
enum DependencyEntry {
    Simple(PackageVersionReq),
    Detailed(DependencyTableEntry),
}

#[derive(Debug, Deserialize)]
struct DependencyTableEntry {
    version: PackageVersionReq,
    #[serde(default)]
    opt: Option<bool>,
    #[serde(default)]
    pin: Option<bool>,
    #[serde(default)]
    git: Option<RemoteGitUrlShorthand>,
    #[serde(default)]
    rev: Option<String>,
}

fn parse_map_to_dependency_vec_opt<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<LuaDependencySpec>>, D::Error>
where
    D: Deserializer<'de>,
{
    let packages: Option<HashMap<PackageName, DependencyEntry>> =
        Option::deserialize(deserializer)?;

    match packages {
        None => Ok(None),
        Some(packages) => Ok(Some(
            packages
                .into_iter()
                .map(|(name, spec)| match spec {
                    DependencyEntry::Simple(version_req) => {
                        Ok(PackageReq { name, version_req }.into())
                    }
                    DependencyEntry::Detailed(entry) => {
                        let source = match (entry.git, entry.rev) {
                            (None, None) => Ok(None),
                            (None, Some(_)) => Err(de::Error::custom(format!(
                                "dependency {} specifies a 'rev', but missing a 'git' field",
                                &name
                            ))),
                            (Some(git), Some(rev)) => Ok(Some(RockSourceSpec::Git(GitSource {
                                url: git.into(),
                                checkout_ref: Some(rev),
                            }))),
                            (Some(git), None) => Ok(Some(RockSourceSpec::Git(GitSource {
                                url: git.into(),
                                checkout_ref: Some(
                                    entry
                                        .version
                                        .clone()
                                        .to_string()
                                        .trim_start_matches("=")
                                        .to_string(),
                                ),
                            }))),
                        }?;
                        Ok(LuaDependencySpec {
                            package_req: PackageReq {
                                name,
                                version_req: entry.version,
                            },
                            opt: OptState::from(entry.opt.unwrap_or(false)),
                            pin: PinnedState::from(entry.pin.unwrap_or(false)),
                            source,
                        })
                    }
                })
                .try_collect()?,
        )),
    }
}

#[derive(Debug, Error)]
pub enum ProjectTomlError {
    #[error("error generating rockspec source:\n{0}")]
    GenerateSource(#[from] GenerateSourceError),
    #[error("error generating rockspec version:\n{0}")]
    GenerateVersion(#[from] GenerateVersionError),
}

#[derive(Debug, Error)]
pub enum LocalProjectTomlValidationError {
    #[error("no lua version provided")]
    NoLuaVersion,
    #[error("could not decode the test spec:\n:{0}")]
    TestSpecError(#[from] TestSpecDecodeError),
    #[error("could not decode the build spec:\n:{0}")]
    BuildSpecInternal(#[from] BuildSpecInternalError),
    #[error(transparent)]
    PlatformValidation(#[from] PlatformValidationError),
    #[error("{}copy_directories cannot contain a rockspec name", ._0.as_ref().map(|p| format!("{p}: ")).unwrap_or_default())]
    CopyDirectoriesContainRockspecName(Option<String>),
    #[error("could not decode the source spec:\n:{0}")]
    RockSource(#[from] RockSourceError),
    #[error("duplicate dependencies: {0}")]
    DuplicateDependencies(PackageNameList),
    #[error("duplicate test dependencies: {0}")]
    DuplicateTestDependencies(PackageNameList),
    #[error("duplicate build dependencies: {0}")]
    DuplicateBuildDependencies(PackageNameList),
    #[error(
        r#"dependencies field cannot contain 'lua'.
        Please provide the version in the top-level 'lua' field

        Example:

        ```toml
        package = "my-package"
        lua = ">=5.1"

        [dependencies]
        # do not add Lua here!
        ```
        "#
    )]
    DependenciesContainLua,
    #[error("error generating rockspec source:\n{0}")]
    GenerateSource(#[from] GenerateSourceError),
    #[error("error generating rockspec version:\n{0}")]
    GenerateVersion(#[from] GenerateVersionError),
}

#[derive(Debug, Error)]
pub enum RemoteProjectTomlValidationError {
    #[error("error generating rockspec source:\n{0}")]
    GenerateSource(#[from] GenerateSourceError),
    #[error("error generating rockspec version:\n{0}")]
    GenerateVersion(#[from] GenerateVersionError),
    #[error(transparent)]
    LocalProjectTomlValidationError(#[from] LocalProjectTomlValidationError),
}

/// The `lux.toml` file.
/// The only required fields are `package` and `build`, which are required to build a project using `lux build`.
/// The rest of the fields are optional, but are required to build a rockspec.
#[derive(Clone, Debug, Deserialize)]
pub struct PartialProjectToml {
    pub(crate) package: PackageName,
    #[serde(default, rename = "version")]
    pub(crate) version_template: PackageVersionTemplate,
    #[serde(default)]
    pub(crate) build: BuildSpecInternal,
    pub(crate) rockspec_format: Option<RockspecFormat>,
    #[serde(default)]
    pub(crate) run: Option<RunSpec>,
    #[serde(default)]
    pub(crate) lua: Option<PackageVersionReq>,
    #[serde(default)]
    pub(crate) description: Option<RockDescription>,
    #[serde(default)]
    pub(crate) supported_platforms: Option<HashMap<PlatformIdentifier, bool>>,
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    pub(crate) dependencies: Option<Vec<LuaDependencySpec>>,
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    pub(crate) build_dependencies: Option<Vec<LuaDependencySpec>>,
    #[serde(default)]
    pub(crate) external_dependencies: Option<HashMap<String, ExternalDependencySpec>>,
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    pub(crate) test_dependencies: Option<Vec<LuaDependencySpec>>,
    #[serde(default, rename = "source")]
    pub(crate) source_template: RockSourceTemplate,
    #[serde(default)]
    pub(crate) test: Option<TestSpecInternal>,
    #[serde(default)]
    pub(crate) deploy: Option<DeploySpec>,

    /// Used to bind the project TOML to a project root
    #[serde(skip, default = "ProjectRoot::new")]
    pub(crate) project_root: ProjectRoot,
}

impl UserData for PartialProjectToml {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, _: ()| Ok(this.package().clone()));
        methods.add_method("to_local", |_, this, _: ()| {
            this.into_local().into_lua_err()
        });
        methods.add_method("to_remote", |_, this, specrev: Option<SpecRev>| {
            this.into_remote(specrev).into_lua_err()
        });
        //TODO:
        //methods.add_method("merge", |_, this, other: PartialLuaRockspec| {
        //    this.merge(other).into_lua_err()
        //});
    }
}

impl HasIntegrity for PartialProjectToml {
    fn hash(&self) -> io::Result<Integrity> {
        let toml_file = self.project_root.join(PROJECT_TOML);
        let content = std::fs::read_to_string(&toml_file)?;
        Ok(Integrity::from(&content))
    }
}

impl PartialProjectToml {
    pub(crate) fn new(str: &str, project_root: ProjectRoot) -> Result<Self, toml::de::Error> {
        Ok(Self {
            project_root,
            ..toml::from_str(str)?
        })
    }

    /// Convert the `PartialProjectToml` struct into a `LocalProjectToml` struct, making
    /// it ready to be used for building a project.
    pub fn into_local(&self) -> Result<LocalProjectToml, LocalProjectTomlValidationError> {
        let project_toml = self.clone();

        // Disallow `lua` to be part of the `dependencies` field
        if project_toml
            .dependencies
            .as_ref()
            .is_some_and(|deps| deps.iter().any(|dep| dep.name() == &"lua".into()))
        {
            return Err(LocalProjectTomlValidationError::DependenciesContainLua);
        }

        let get_duplicates = |dependencies: &Option<Vec<LuaDependencySpec>>| {
            dependencies
                .iter()
                .flat_map(|deps| {
                    deps.iter()
                        .map(|dep| dep.package_req().name())
                        .duplicates()
                        .cloned()
                })
                .collect_vec()
        };
        let duplicate_dependencies = get_duplicates(&self.dependencies);
        if !duplicate_dependencies.is_empty() {
            return Err(LocalProjectTomlValidationError::DuplicateDependencies(
                PackageNameList::new(duplicate_dependencies),
            ));
        }
        let duplicate_test_dependencies = get_duplicates(&self.test_dependencies);
        if !duplicate_test_dependencies.is_empty() {
            return Err(LocalProjectTomlValidationError::DuplicateTestDependencies(
                PackageNameList::new(duplicate_test_dependencies),
            ));
        }
        let duplicate_build_dependencies = get_duplicates(&self.build_dependencies);
        if !duplicate_build_dependencies.is_empty() {
            return Err(LocalProjectTomlValidationError::DuplicateBuildDependencies(
                PackageNameList::new(duplicate_build_dependencies),
            ));
        }

        let validated = LocalProjectToml {
            internal: project_toml.clone(),

            package: project_toml.package,
            version: project_toml
                .version_template
                .try_generate(&self.project_root, None)
                .unwrap_or(PackageVersion::default_dev_version()),
            lua: project_toml
                .lua
                .ok_or(LocalProjectTomlValidationError::NoLuaVersion)?,
            description: project_toml.description.unwrap_or_default(),
            run: project_toml.run.map(PerPlatform::new),
            supported_platforms: PlatformSupport::parse(
                &project_toml
                    .supported_platforms
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(platform, supported)| {
                        if supported {
                            format!("{platform}")
                        } else {
                            format!("!{platform}")
                        }
                    })
                    .collect_vec(),
            )?,
            // Merge dependencies internally with lua version
            // so the output of `dependencies()` is consistent
            dependencies: PerPlatform::new(project_toml.dependencies.unwrap_or_default()),
            build_dependencies: PerPlatform::new(
                project_toml.build_dependencies.unwrap_or_default(),
            ),
            external_dependencies: PerPlatform::new(
                project_toml.external_dependencies.unwrap_or_default(),
            ),
            test_dependencies: PerPlatform::new(project_toml.test_dependencies.unwrap_or_default()),
            test: PerPlatform::new(TestSpec::try_from(
                project_toml.test.clone().unwrap_or_default(),
            )?),
            build: PerPlatform::new(BuildSpec::from_internal_spec(project_toml.build.clone())?),
            deploy: PerPlatform::new(project_toml.deploy.clone().unwrap_or_default()),
            rockspec_format: project_toml.rockspec_format.clone(),

            source: PerPlatform::new(RemoteRockSource {
                local: LocalRockSource::default(),
                source_spec: RockSourceSpec::File(self.project_root.to_path_buf()),
            }),
        };

        let rockspec_file_name = format!("{}-{}.rockspec", validated.package, validated.version);

        if validated
            .build
            .default
            .copy_directories
            .contains(&PathBuf::from(&rockspec_file_name))
        {
            return Err(LocalProjectTomlValidationError::CopyDirectoriesContainRockspecName(None));
        }

        for (platform, build_override) in &validated.build.per_platform {
            if build_override
                .copy_directories
                .contains(&PathBuf::from(&rockspec_file_name))
            {
                return Err(
                    LocalProjectTomlValidationError::CopyDirectoriesContainRockspecName(Some(
                        platform.to_string(),
                    )),
                );
            }
        }

        Ok(validated)
    }

    /// Convert the `PartialProjectToml` struct into a `RemoteProjectToml` struct, making
    /// it ready to be serialized into a rockspec.
    /// A source must be provided for the rockspec to be valid.
    pub fn into_remote(
        &self,
        specrev: Option<SpecRev>,
    ) -> Result<RemoteProjectToml, RemoteProjectTomlValidationError> {
        let version = self
            .version_template
            .try_generate(&self.project_root, specrev)?;
        let source =
            self.source_template
                .try_generate(&self.project_root, &self.package, &version)?;
        let source = PerPlatform::new(RemoteRockSource::try_from(source).map_err(|err| {
            RemoteProjectTomlValidationError::LocalProjectTomlValidationError(
                LocalProjectTomlValidationError::RockSource(err),
            )
        })?);
        let mut local = self.into_local()?;
        local.version = version;

        let validated = RemoteProjectToml { source, local };

        Ok(validated)
    }

    // In the not-yet-validated struct, we create getters only
    // for the non-optional fields.
    pub fn package(&self) -> &PackageName {
        &self.package
    }

    /// Returns the current package version, which may be generated from a template
    pub fn version(&self) -> Result<PackageVersion, GenerateVersionError> {
        self.version_template.try_generate(&self.project_root, None)
    }

    /// Merge the `ProjectToml` struct with an unvalidated `LuaRockspec`.
    /// The final merged struct can then be validated.
    pub fn merge(self, other: PartialLuaRockspec) -> Self {
        PartialProjectToml {
            package: other.package.unwrap_or(self.package),
            version_template: self.version_template,
            lua: other
                .dependencies
                .as_ref()
                .and_then(|deps| {
                    deps.iter()
                        .find(|dep| dep.name() == &"lua".into())
                        .and_then(|dep| {
                            if dep.version_req().is_any() {
                                None
                            } else {
                                Some(dep.version_req().clone())
                            }
                        })
                })
                .or(self.lua),
            build: other.build.unwrap_or(self.build),
            run: self.run,
            description: other.description.or(self.description),
            supported_platforms: other
                .supported_platforms
                .map(|platform_support| platform_support.platforms().clone())
                .or(self.supported_platforms),
            dependencies: other
                .dependencies
                .map(|deps| {
                    deps.into_iter()
                        .filter(|dep| dep.name() != &"lua".into())
                        .collect()
                })
                .or(self.dependencies),
            build_dependencies: other.build_dependencies.or(self.build_dependencies),
            test_dependencies: other.test_dependencies.or(self.test_dependencies),
            external_dependencies: other.external_dependencies.or(self.external_dependencies),
            source_template: self.source_template,
            test: other.test.or(self.test),
            deploy: other.deploy.or(self.deploy),
            rockspec_format: other.rockspec_format.or(self.rockspec_format),

            // Keep the project root the same, as it is not part of the lua rockspec
            project_root: self.project_root,
        }
    }
}

// This is automatically implemented for `RemoteProjectToml`,
// but we also add a special implementation for `ProjectToml` (as providing a lua version
// is required even by the non-validated struct).
impl LuaVersionCompatibility for PartialProjectToml {
    fn validate_lua_version(&self, version: &LuaVersion) -> Result<(), LuaVersionError> {
        if self.supports_lua_version(version) {
            Ok(())
        } else {
            Err(LuaVersionError::LuaVersionUnsupported(
                version.clone(),
                self.package().to_owned(),
                self.version_template
                    .try_generate(&self.project_root, None)
                    .unwrap_or(PackageVersion::default_dev_version()),
            ))
        }
    }

    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), LuaVersionError> {
        let _ = self.lua_version_matches(config)?;
        Ok(())
    }

    fn lua_version_matches(&self, config: &Config) -> Result<LuaVersion, LuaVersionError> {
        let version = LuaVersion::from(config)?.clone();
        if self.supports_lua_version(&version) {
            Ok(version)
        } else {
            Err(LuaVersionError::LuaVersionUnsupported(
                version,
                self.package.clone(),
                self.version_template
                    .try_generate(&self.project_root, None)
                    .unwrap_or(PackageVersion::default_dev_version()),
            ))
        }
    }

    fn supports_lua_version(&self, lua_version: &LuaVersion) -> bool {
        self.lua
            .as_ref()
            .is_none_or(|lua| lua.matches(&lua_version.as_version()))
    }

    fn lua_version(&self) -> Option<LuaVersion> {
        for (possibility, version) in [
            ("5.5.0", LuaVersion::Lua55),
            ("5.4.0", LuaVersion::Lua54),
            ("5.3.0", LuaVersion::Lua53),
            ("5.2.0", LuaVersion::Lua52),
            ("5.1.0", LuaVersion::Lua51),
        ] {
            let possibility = unsafe { possibility.parse().unwrap_unchecked() };
            if self
                .lua
                .as_ref()
                .is_none_or(|lua| lua.matches(&possibility))
            {
                return Some(version);
            }
        }
        None
    }
}

// TODO(vhyrro): Move this struct into a different directory.
#[derive(Debug, Clone, Deserialize)]
pub struct RunSpec {
    /// The command to execute when running the project
    pub(crate) command: Option<RunCommand>,
    /// Arguments to pass to the command
    pub(crate) args: Option<NonEmpty<String>>,
}

/// The `lux.toml` file, after being properly deserialized.
/// This struct may be used to build a local version of a project.
/// To build a rockspec, use `RemoteProjectToml`.
#[derive(Debug)]
pub struct LocalProjectToml {
    package: PackageName,
    version: PackageVersion,
    lua: PackageVersionReq,
    rockspec_format: Option<RockspecFormat>,
    run: Option<PerPlatform<RunSpec>>,
    description: RockDescription,
    supported_platforms: PlatformSupport,
    dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    build_dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    external_dependencies: PerPlatform<HashMap<String, ExternalDependencySpec>>,
    test_dependencies: PerPlatform<Vec<LuaDependencySpec>>,
    test: PerPlatform<TestSpec>,
    build: PerPlatform<BuildSpec>,
    deploy: PerPlatform<DeploySpec>,

    // Used for simpler serialization
    internal: PartialProjectToml,

    /// A source pointing to the current project's root.
    source: PerPlatform<RemoteRockSource>,
}

impl LocalProjectToml {
    pub fn run(&self) -> Option<&PerPlatform<RunSpec>> {
        self.run.as_ref()
    }

    /// Convert this project TOML to a Lua rockspec.
    /// Fails if there is no valid project root or if there are off-spec dependencies.
    pub fn to_lua_rockspec(&self) -> Result<LocalLuaRockspec, LuaRockspecError> {
        if let Some(dep) = self
            .dependencies()
            .per_platform
            .iter()
            .filter_map(|(_, deps)| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(LuaRockspecError::OffSpecDependency(dep.name().clone()));
        }
        if let Some(dep) = self
            .build_dependencies()
            .per_platform
            .iter()
            .filter_map(|(_, deps)| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(LuaRockspecError::OffSpecBuildDependency(dep.name().clone()));
        }
        if let Some(dep) = self
            .test_dependencies()
            .per_platform
            .iter()
            .filter_map(|(_, deps)| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(LuaRockspecError::OffSpecTestDependency(dep.name().clone()));
        }
        LocalLuaRockspec::new(
            &self.to_lua_remote_rockspec_string()?,
            self.internal.project_root.clone(),
        )
    }
}

impl Rockspec for LocalProjectToml {
    type Error = ProjectTomlError;

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

    fn build_mut(&mut self) -> &mut PerPlatform<BuildSpec> {
        &mut self.build
    }

    fn test_mut(&mut self) -> &mut PerPlatform<TestSpec> {
        &mut self.test
    }

    fn format(&self) -> &Option<RockspecFormat> {
        &self.rockspec_format
    }

    fn source(&self) -> &PerPlatform<RemoteRockSource> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut PerPlatform<RemoteRockSource> {
        &mut self.source
    }

    fn deploy(&self) -> &PerPlatform<DeploySpec> {
        &self.deploy
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<DeploySpec> {
        &mut self.deploy
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, Self::Error> {
        let project_root = &self.internal.project_root;
        let version = self
            .internal
            .version_template
            .try_generate(project_root, None)?;
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.rockspec_format
                .as_ref()
                .unwrap_or(&RockspecFormat::default()),
            self.package,
            &version
        );

        let mut template = Vec::new();

        if self.description != RockDescription::default() {
            template.push(self.description.display_lua());
        }

        if self.supported_platforms != PlatformSupport::default() {
            template.push(self.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.internal.dependencies.clone().unwrap_or_default();
            dependencies.insert(
                0,
                PackageReq {
                    name: "lua".into(),
                    version_req: self.lua.clone(),
                }
                .into(),
            );
            template.push(Dependencies(&dependencies).display_lua());
        }

        let mut build_dependencies = self
            .internal
            .build_dependencies
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let build_backend_dependency = self
            .internal
            .build
            .build_type
            .as_ref()
            .and_then(|build_type| build_type.luarocks_build_backend());

        if let Some(build_backend_dependency) = build_backend_dependency {
            build_dependencies.push(build_backend_dependency);
        }

        if !build_dependencies.is_empty() {
            template.push(BuildDependencies(&build_dependencies).display_lua());
        }

        match self.internal.external_dependencies {
            Some(ref external_dependencies) if !external_dependencies.is_empty() => {
                template.push(ExternalDependencies(external_dependencies).display_lua());
            }
            _ => {}
        }

        let source =
            self.internal
                .source_template
                .try_generate(project_root, &self.package, &version)?;
        template.push(source.display_lua());

        template.push(self.internal.build.display_lua());

        let unformatted_code = std::iter::once(starter)
            .chain(template.into_iter().map(|kv| kv.to_string()))
            .join("\n\n");
        let result = match stylua_lib::format_code(
            &unformatted_code,
            stylua_lib::Config::default(),
            None,
            stylua_lib::OutputVerification::Full,
        ) {
            Ok(formatted_code) => formatted_code,
            Err(_) => unformatted_code,
        };
        Ok(result)
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ProjectTomlIntegrityError {
    LuaRockspecError(#[from] LuaRockspecError),
    IoError(#[from] io::Error),
}

impl HasIntegrity for LocalProjectToml {
    fn hash(&self) -> io::Result<Integrity> {
        match self.to_lua_rockspec() {
            Ok(lua_rockspec) => lua_rockspec.hash(),
            Err(_) => self.internal.hash(),
        }
    }
}

#[derive(Debug)]
pub struct RemoteProjectToml {
    local: LocalProjectToml,
    source: PerPlatform<RemoteRockSource>,
}

impl RemoteProjectToml {
    pub fn to_lua_rockspec(&self) -> Result<RemoteLuaRockspec, LuaRockspecError> {
        RemoteLuaRockspec::new(&self.to_lua_remote_rockspec_string()?)
    }
}

impl Rockspec for RemoteProjectToml {
    type Error = ProjectTomlError;

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
        self.local.build_dependencies()
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

    fn build_mut(&mut self) -> &mut PerPlatform<BuildSpec> {
        self.local.build_mut()
    }

    fn test_mut(&mut self) -> &mut PerPlatform<TestSpec> {
        self.local.test_mut()
    }

    fn format(&self) -> &Option<RockspecFormat> {
        self.local.format()
    }

    fn source(&self) -> &PerPlatform<RemoteRockSource> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut PerPlatform<RemoteRockSource> {
        &mut self.source
    }

    fn deploy(&self) -> &PerPlatform<DeploySpec> {
        self.local.deploy()
    }

    fn deploy_mut(&mut self) -> &mut PerPlatform<DeploySpec> {
        self.local.deploy_mut()
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, Self::Error> {
        let project_root = &self.local.internal.project_root;
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.local
                .rockspec_format
                .as_ref()
                .unwrap_or(&RockspecFormat::default()),
            self.local.package,
            self.version()
        );

        let mut template = Vec::new();

        if self.local.description != RockDescription::default() {
            template.push(self.local.description.display_lua());
        }

        if self.local.supported_platforms != PlatformSupport::default() {
            template.push(self.local.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.local.internal.dependencies.clone().unwrap_or_default();
            dependencies.insert(
                0,
                PackageReq {
                    name: "lua".into(),
                    version_req: self.local.lua.clone(),
                }
                .into(),
            );
            template.push(Dependencies(&dependencies).display_lua());
        }

        let mut build_dependencies = self
            .local
            .internal
            .build_dependencies
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let build_backend_dependency = self
            .local
            .internal
            .build
            .build_type
            .as_ref()
            .and_then(|build_type| build_type.luarocks_build_backend());

        if let Some(build_backend_dependency) = build_backend_dependency {
            build_dependencies.push(build_backend_dependency);
        }

        if !build_dependencies.is_empty() {
            template.push(BuildDependencies(&build_dependencies).display_lua());
        }

        match self.local.internal.external_dependencies {
            Some(ref external_dependencies) if !external_dependencies.is_empty() => {
                template.push(ExternalDependencies(external_dependencies).display_lua());
            }
            _ => {}
        }

        let source = self.local.internal.source_template.try_generate(
            project_root,
            &self.local.internal.package,
            self.version(),
        )?;
        template.push(source.display_lua());

        if let Some(ref deploy) = self.local.internal.deploy {
            template.push(deploy.display_lua());
        }

        template.push(self.local.internal.build.display_lua());

        let unformatted_code = std::iter::once(starter)
            .chain(template.into_iter().map(|kv| kv.to_string()))
            .join("\n\n");
        let result = match stylua_lib::format_code(
            &unformatted_code,
            stylua_lib::Config::default(),
            None,
            stylua_lib::OutputVerification::Full,
        ) {
            Ok(formatted_code) => formatted_code,
            Err(_) => unformatted_code,
        };
        Ok(result)
    }
}

impl HasIntegrity for RemoteProjectToml {
    fn hash(&self) -> io::Result<Integrity> {
        self.to_lua_rockspec()
            .map_err(|err| {
                io::Error::other(format!(
                    "unable to convert remote project to Lua rockspec:\n{}",
                    err
                ))
            })?
            .hash()
    }
}

impl UserData for LocalProjectToml {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, _: ()| Ok(this.package().clone()));
        methods.add_method("version", |_, this, _: ()| Ok(this.version().clone()));
        methods.add_method("description", |_, this, _: ()| {
            Ok(this.description().clone())
        });
        methods.add_method("supported_platforms", |_, this, _: ()| {
            Ok(this.supported_platforms().clone())
        });
        methods.add_method("dependencies", |_, this, _: ()| {
            Ok(this.dependencies().clone())
        });
        methods.add_method("build_dependencies", |_, this, _: ()| {
            Ok(this.build_dependencies().clone())
        });
        methods.add_method("external_dependencies", |_, this, _: ()| {
            Ok(this.external_dependencies().clone())
        });
        methods.add_method("test_dependencies", |_, this, _: ()| {
            Ok(this.test_dependencies().clone())
        });
        methods.add_method("build", |_, this, _: ()| Ok(this.build().clone()));
        methods.add_method("test", |_, this, _: ()| Ok(this.test().clone()));
        methods.add_method("format", |_, this, _: ()| Ok(this.format().clone()));
        methods.add_method("source", |_, this, _: ()| Ok(this.source().clone()));
        methods.add_method("to_lua_rockspec_string", |_, this, _: ()| {
            this.to_lua_remote_rockspec_string()
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
        methods.add_method("to_lua_rockspec", |_, this, _: ()| {
            this.to_lua_rockspec().into_lua_err()
        });
    }
}

impl UserData for RemoteProjectToml {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, _: ()| Ok(this.package().clone()));
        methods.add_method("version", |_, this, _: ()| Ok(this.version().clone()));
        methods.add_method("description", |_, this, _: ()| {
            Ok(this.description().clone())
        });
        methods.add_method("supported_platforms", |_, this, _: ()| {
            Ok(this.supported_platforms().clone())
        });
        methods.add_method("dependencies", |_, this, _: ()| {
            Ok(this.dependencies().clone())
        });
        methods.add_method("build_dependencies", |_, this, _: ()| {
            Ok(this.build_dependencies().clone())
        });
        methods.add_method("external_dependencies", |_, this, _: ()| {
            Ok(this.external_dependencies().clone())
        });
        methods.add_method("test_dependencies", |_, this, _: ()| {
            Ok(this.test_dependencies().clone())
        });
        methods.add_method("build", |_, this, _: ()| Ok(this.build().clone()));
        methods.add_method("test", |_, this, _: ()| Ok(this.test().clone()));
        methods.add_method("format", |_, this, _: ()| Ok(this.format().clone()));
        methods.add_method("source", |_, this, _: ()| Ok(this.source().clone()));
        methods.add_method("to_lua_rockspec_string", |_, this, _: ()| {
            this.to_lua_remote_rockspec_string()
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))
        });
        methods.add_method("to_lua_rockspec", |_, this, _: ()| {
            this.to_lua_rockspec().into_lua_err()
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use assert_fs::prelude::{PathChild, PathCopy, PathCreateDir};
    use git2::{Repository, RepositoryInitOptions};
    use url::Url;

    use crate::{
        git::{url::RemoteGitUrl, GitSource},
        lua_rockspec::{PartialLuaRockspec, PerPlatform, RemoteLuaRockspec, RockSourceSpec},
        project::{Project, ProjectRoot},
        rockspec::{lua_dependency::LuaDependencySpec, Rockspec},
    };

    use super::PartialProjectToml;

    #[test]
    fn project_toml_parsing() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.3"

        rockspec_format = "1.0"

        [source]
        url = "https://example.com"

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [run]
        args = ["--foo", "--bar"]

        [build]
        type = "builtin"
        "#;

        let project = PartialProjectToml::new(project_toml, ProjectRoot::default()).unwrap();
        let _ = project.into_remote(None).unwrap();

        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "69.420"

        [source]
        url = "https://example.com"
        hash = "sha256-di00mD8txN7rjaVpvxzNbnQsAh6H16zUtJZapH7U4HU="
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = ["--foo", "--bar"]

        [build]
        type = "builtin"
        "#;

        let project = PartialProjectToml::new(project_toml, ProjectRoot::default()).unwrap();
        let _ = project.into_remote(None).unwrap();
    }

    #[test]
    fn compare_project_toml_with_rockspec() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        # For testing, specify a custom rockspec format
        # (defaults to 3.0)
        rockspec_format = "1.0"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "1.0"

        [source]
        url = "https://example.com"
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = ["--foo", "--bar"]

        [deploy]
        wrap_bin_scripts = false

        [build]
        type = "builtin"

        [build.install.lua]
        "foo.bar" = "src/bar.lua"

        [build.install.lib]
        "foo.baz" = "src/baz.c"

        [build.install.bin]
        "bla" = "src/bla"

        [build.install.conf]
        "cfg.conf" = "resources/config.conf"
        "#;

        let expected_rockspec = r#"
            rockspec_format = "1.0"
            package = "my-package"
            version = "1.0.0"

            source = {
                url = "https://example.com",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }

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
                "lua ==5.1",
                "foo ==1.0",
                "bar >=2.0",
            }

            build_dependencies = {
                "baz ==1.0",
            }

            external_dependencies = {
                foo = { header = "foo.h" },
                bar = { library = "libbar.so" },
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

            deploy = {
                wrap_bin_scripts = false,
            }

            build = {
                type = "builtin",
                install = {
                    lua = {
                        ["foo.bar"] = "src/bar.lua",
                    },
                    lib = {
                        ["foo.baz"] = "src/baz.c",
                    },
                    bin = {
                        bla = "src/bla",
                    },
                    conf = {
                        ["cfg.conf"] = "resources/config.conf",
                    },
                },
            }
        "#;

        let expected_rockspec = RemoteLuaRockspec::new(expected_rockspec).unwrap();

        let project_toml = PartialProjectToml::new(project_toml, ProjectRoot::default()).unwrap();
        let rockspec = project_toml
            .into_remote(None)
            .unwrap()
            .to_lua_rockspec()
            .unwrap();

        let sorted_package_reqs = |v: &PerPlatform<Vec<LuaDependencySpec>>| {
            let mut v = v.current_platform().clone();
            v.sort_by(|a, b| a.name().cmp(b.name()));
            v
        };

        assert_eq!(rockspec.package(), expected_rockspec.package());
        assert_eq!(rockspec.version(), expected_rockspec.version());
        assert_eq!(rockspec.description(), expected_rockspec.description());
        assert_eq!(
            rockspec.supported_platforms(),
            expected_rockspec.supported_platforms()
        );
        assert_eq!(
            sorted_package_reqs(rockspec.dependencies()),
            sorted_package_reqs(expected_rockspec.dependencies())
        );
        assert_eq!(
            sorted_package_reqs(rockspec.build_dependencies()),
            sorted_package_reqs(expected_rockspec.build_dependencies())
        );
        assert_eq!(
            rockspec.external_dependencies(),
            expected_rockspec.external_dependencies()
        );
        assert_eq!(rockspec.source(), expected_rockspec.source());
        assert_eq!(rockspec.build(), expected_rockspec.build());
        assert_eq!(rockspec.format(), expected_rockspec.format());
    }

    #[test]
    fn merge_project_toml_with_partial_rockspec() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        # For testing, specify a custom rockspec format
        # (defaults to 3.0)
        rockspec_format = "1.0"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "1.0"

        [source]
        url = "https://example.com"
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = [ "--foo", "--bar" ]

        [build]
        type = "builtin"
        "#;

        let mergable_rockspec_content = r#"
            rockspec_format = "1.0"
            package = "my-package-overwritten"

            description = {
                summary = "A summary overwritten",
                detailed = "A detailed description overwritten",
                license = "GPL-2.0",
                homepage = "https://example.com/overwritten",
                issues_url = "https://example.com/issues/overwritten",
                maintainer = "John Doe Overwritten",
                labels = {"over", "written"},
            }

            -- Inverted supported platforms
            supported_platforms = {"!linux", "windows"}

            dependencies = {
                "lua 5.1",
                "foo >1.0",
                "bar <=2.0",
            }

            build_dependencies = {
                "baz >1.0",
            }

            external_dependencies = {
                foo = { header = "overwritten.h" },
                bar = { library = "overwritten.so" },
            }

            test = {
                type = "command",
                script = "overwritten.lua",
                flags = {"over", "written"},
            }

            build = {
                type = "builtin",
            }
        "#;

        let remote_rockspec_content = format!(
            r#"{}
            version = "1.0.0"
            source = {{
                url = "https://example.com",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }}
        "#,
            &mergable_rockspec_content
        );

        let project_toml = PartialProjectToml::new(project_toml, ProjectRoot::default()).unwrap();
        let partial_rockspec = PartialLuaRockspec::new(mergable_rockspec_content).unwrap();
        let expected_rockspec = RemoteLuaRockspec::new(&remote_rockspec_content).unwrap();

        let merged = project_toml
            .merge(partial_rockspec)
            .into_remote(None)
            .unwrap();

        let sorted_package_reqs = |v: &PerPlatform<Vec<LuaDependencySpec>>| {
            let mut v = v.current_platform().clone();
            v.sort_by(|a, b| a.name().cmp(b.name()));
            v
        };

        assert_eq!(merged.package(), expected_rockspec.package());
        assert_eq!(merged.version(), expected_rockspec.version());
        assert_eq!(merged.description(), expected_rockspec.description());
        assert_eq!(
            merged.supported_platforms(),
            expected_rockspec.supported_platforms()
        );
        assert_eq!(
            sorted_package_reqs(merged.dependencies()),
            sorted_package_reqs(expected_rockspec.dependencies())
        );
        assert_eq!(
            sorted_package_reqs(merged.build_dependencies()),
            sorted_package_reqs(expected_rockspec.build_dependencies())
        );
        assert_eq!(
            merged.external_dependencies(),
            expected_rockspec.external_dependencies()
        );
        assert_eq!(merged.source(), expected_rockspec.source());
        assert_eq!(merged.build(), expected_rockspec.build());
        assert_eq!(merged.format(), expected_rockspec.format());
        // Ensure that the run command is retained after merge.
        assert!(merged.local.run().is_some());
    }

    #[test]
    fn project_toml_with_lua_in_dependencies() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        # lua = ">5.1"

        [dependencies]
        lua = "5.1" # disallowed

        [build]
        type = "builtin"
        "#;

        PartialProjectToml::new(project_toml, ProjectRoot::default())
            .unwrap()
            .into_local()
            .unwrap_err();
    }

    #[test]
    fn project_toml_with_invalid_run_command() {
        for command in ["lua", "lua5.1", "lua5.2", "lua5.3", "lua5.4", "luajit"] {
            let project_toml = format!(
                r#"
                package = "my-package"
                version = "1.0.0"
                lua = "5.1"

                [build]
                type = "builtin"

                [run]
                command = "{command}"
                "#,
            );

            PartialProjectToml::new(&project_toml, ProjectRoot::default()).unwrap_err();
        }
    }

    #[test]
    fn generate_non_deterministic_git_source() {
        let rockspec_content = r#"
            package = "test-package"
            version = "1.0.0"
            lua = ">=5.1"

            [source]
            url = "git+https://exaple.com/repo.git"

            [build]
            type = "builtin"
        "#;

        PartialProjectToml::new(rockspec_content, ProjectRoot::default())
            .unwrap()
            .into_remote(None)
            .unwrap_err();
    }

    #[test]
    fn generate_deterministic_git_source() {
        let rockspec_content = r#"
            package = "test-package"
            version = "1.0.0"
            lua = ">=5.1"

            [source]
            url = "git+https://exaple.com/owner/repo.git"
            tag = "v0.1.0"

            [build]
            type = "builtin"
        "#;

        PartialProjectToml::new(rockspec_content, ProjectRoot::default())
            .unwrap()
            .into_remote(None)
            .unwrap();
    }

    fn init_sample_project_repo(temp_dir: &assert_fs::TempDir) -> Repository {
        let sample_project: PathBuf = "resources/test/sample-projects/source-template/".into();
        temp_dir.copy_from(&sample_project, &["**"]).unwrap();
        let repo = Repository::init(temp_dir).unwrap();
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head("main");
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "name").unwrap();
            config.set_str("user.email", "email").unwrap();
            let mut index = repo.index().unwrap();
            let id = index.write_tree().unwrap();

            let tree = repo.find_tree(id).unwrap();
            let sig = repo.signature().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial\n\nbody", &tree, &[])
                .unwrap();
        }
        repo
    }

    fn create_tag(repo: &Repository, name: &str) {
        let sig = repo.signature().unwrap();
        let id = repo.head().unwrap().target().unwrap();
        let obj = repo.find_object(id, None).unwrap();
        repo.tag(name, &obj, &sig, "msg", true).unwrap();
    }

    #[test]
    fn test_git_project_generate_dev_source() {
        let project_root = assert_fs::TempDir::new().unwrap();
        init_sample_project_repo(&project_root);
        let project = Project::from(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &RockSourceSpec::Git { .. }));
        if let RockSourceSpec::Git(GitSource { url, checkout_ref }) = source_spec {
            let expected_url: RemoteGitUrl =
                "https://github.com/lumen-oss/lux.git".parse().unwrap();
            assert_eq!(url, &expected_url);
            assert!(checkout_ref.is_some());
        }
        assert_eq!(source.unpack_dir, Some("lux-dev".into()));
    }

    #[test]
    fn test_git_project_generate_non_semver_tag_source() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        let tag_name = "bla";
        create_tag(&repo, tag_name);
        let project = Project::from(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &RockSourceSpec::Git { .. }));
        if let RockSourceSpec::Git(GitSource { url, checkout_ref }) = source_spec {
            let expected_url: RemoteGitUrl =
                "https://github.com/lumen-oss/lux.git".parse().unwrap();
            assert_eq!(url, &expected_url);
            assert_eq!(checkout_ref, &Some(tag_name.to_string()));
        }
        assert_eq!(source.unpack_dir, Some("lux-dev".into()));
    }

    #[test]
    fn test_git_project_generate_release_source_tag_with_v_prefix() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        let tag_name = "v1.0.0";
        create_tag(&repo, "bla");
        create_tag(&repo, tag_name);
        let project = Project::from(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &RockSourceSpec::Url { .. }));
        if let RockSourceSpec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/v1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }

    #[test]
    fn test_git_project_generate_release_source_tag_without_v_prefix() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        create_tag(&repo, "bla");
        let tag_name = "1.0.0";
        create_tag(&repo, tag_name);
        let project = Project::from(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &RockSourceSpec::Url { .. }));
        if let RockSourceSpec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }

    #[test]
    fn test_git_project_in_subdirectory() {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let sample_project: PathBuf = "resources/test/sample-projects/source-template/".into();
        let project_dir = temp_dir.child("lux");
        project_dir.create_dir_all().unwrap();
        project_dir.copy_from(&sample_project, &["**"]).unwrap();
        let repo = Repository::init(&temp_dir).unwrap();
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head("main");
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "name").unwrap();
            config.set_str("user.email", "email").unwrap();
            let mut index = repo.index().unwrap();
            let id = index.write_tree().unwrap();

            let tree = repo.find_tree(id).unwrap();
            let sig = repo.signature().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial\n\nbody", &tree, &[])
                .unwrap();
        }
        create_tag(&repo, "bla");
        let tag_name = "1.0.0";
        create_tag(&repo, tag_name);
        let project = Project::from(&project_dir).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &RockSourceSpec::Url { .. }));
        if let RockSourceSpec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }
}
