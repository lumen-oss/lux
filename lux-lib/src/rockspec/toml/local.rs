use std::collections::HashMap;
use std::io;

use itertools::Itertools;
use ssri::Integrity;

use crate::hash::HasIntegrity;
use crate::package;
use crate::package::{BuildDependencies, Dependencies};
use crate::rockspec::{self, display::AsLuaKV, Rockspec};

use super::partial::{Partial, RunSpec};
use super::{build_spec_with_autodetected_modules, validate_generated_lua, Error};

/// The `lux.toml` file, after being properly deserialized.
/// This struct may be used to build a local version of a project.
/// To build a rockspec, use `Remote`.
#[derive(Debug)]
pub struct Local {
    pub(super) package: package::Name,
    pub(super) version: package::Version,
    pub(super) lua: package::VersionReq,
    pub(super) rockspec_format: Option<rockspec::Format>,
    pub(super) run: Option<rockspec::PerPlatform<RunSpec>>,
    pub(super) description: rockspec::description::Spec,
    pub(super) supported_platforms: rockspec::platform::Support,
    pub(super) dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    pub(super) build_dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    pub(super) external_dependencies:
        rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>>,
    pub(super) test_dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    pub(super) test: rockspec::PerPlatform<rockspec::test::Spec>,
    pub(super) build: rockspec::PerPlatform<rockspec::build::Spec>,
    pub(super) deploy: rockspec::PerPlatform<rockspec::deploy::Spec>,

    pub(super) internal: Partial,

    /// A source pointing to the current project's root.
    pub(super) source: rockspec::PerPlatform<rockspec::source::Remote>,
}

impl Local {
    pub fn run(&self) -> Option<&rockspec::PerPlatform<RunSpec>> {
        self.run.as_ref()
    }

    /// Convert this project TOML to a Lua rockspec.
    /// Fails if there is no valid project root or if there are off-spec dependencies.
    pub fn to_lua_rockspec(&self) -> Result<rockspec::lua::Local, rockspec::Error> {
        if let Some(dep) = self
            .dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecSpec(dep.name().clone()).into());
        }
        if let Some(dep) = self
            .build_dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecBuildSpec(dep.name().clone()).into());
        }
        if let Some(dep) = self
            .test_dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecTestSpec(dep.name().clone()).into());
        }
        Ok(rockspec::lua::Local::new(
            &self.to_lua_remote_rockspec_string()?,
            self.internal.lux_toml_dir.clone(),
        )?)
    }

    /// Generates this project's rockspec for building it in place.
    ///
    /// Uses the remote source when one can be generated,
    /// so that the rockspec stored in the install tree points at the published source.
    /// The returned rockspec's effective source points at the project root.
    pub fn to_build_rockspec(&self) -> Result<rockspec::lua::Remote, rockspec::Error> {
        let content = match self.to_lua_remote_rockspec_string() {
            Ok(content) => content,
            Err(_) => self.to_lua_local_rockspec_string()?,
        };
        Ok(rockspec::lua::Local::from_project_toml(self, content)
            .into_remote_with_source(self.source.clone()))
    }

    /// Like [`Rockspec::to_lua_remote_rockspec_string`], but pointing at the project root,
    /// so projects without a release URL can still render a rockspec.
    fn to_lua_local_rockspec_string(&self) -> Result<String, Error> {
        self.render_rockspec(
            &self.version,
            Some(self.source.current_platform().source_spec.display_lua()),
        )
    }

    fn render_rockspec(
        &self,
        version: &package::Version,
        source: Option<rockspec::display::LuaKV>,
    ) -> Result<String, Error> {
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.rockspec_format
                .as_ref()
                .unwrap_or(&rockspec::Format::default()),
            self.package,
            version
        );

        let mut template = Vec::new();

        if self.description != rockspec::description::Spec::default() {
            template.push(self.description.display_lua());
        }

        if self.supported_platforms != rockspec::platform::Support::default() {
            template.push(self.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.internal.dependencies.clone().unwrap_or_default();
            if !self.lua.is_luau() {
                dependencies.insert(
                    0,
                    package::Req {
                        name: "lua".into(),
                        version_req: self.lua.clone(),
                    }
                    .into(),
                );
            }
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
                template.push(
                    rockspec::external_dependency::Dependencies(external_dependencies)
                        .display_lua(),
                );
            }
            _ => {}
        }

        if let Some(source) = source {
            template.push(source);
        }

        template.push(
            build_spec_with_autodetected_modules(&self.internal.build, &self.internal.lux_toml_dir)
                .display_lua(),
        );

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
        validate_generated_lua(&result)?;
        Ok(result)
    }
}

impl Rockspec for Local {
    fn package(&self) -> &package::Name {
        &self.package
    }

    fn version(&self) -> &package::Version {
        &self.version
    }

    fn description(&self) -> &rockspec::description::Spec {
        &self.description
    }

    fn supported_platforms(&self) -> &rockspec::platform::Support {
        &self.supported_platforms
    }

    fn lua(&self) -> &package::VersionReq {
        &self.lua
    }

    fn dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.dependencies
    }

    fn build_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.build_dependencies
    }

    fn external_dependencies(
        &self,
    ) -> &rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>> {
        &self.external_dependencies
    }

    fn test_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.test_dependencies
    }

    fn build(&self) -> &rockspec::PerPlatform<rockspec::build::Spec> {
        &self.build
    }

    fn test(&self) -> &rockspec::PerPlatform<rockspec::test::Spec> {
        &self.test
    }

    fn build_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::build::Spec> {
        &mut self.build
    }

    fn test_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::test::Spec> {
        &mut self.test
    }

    fn format(&self) -> &Option<rockspec::Format> {
        &self.rockspec_format
    }

    fn source(&self) -> &rockspec::PerPlatform<rockspec::source::Remote> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::source::Remote> {
        &mut self.source
    }

    fn deploy(&self) -> &rockspec::PerPlatform<rockspec::deploy::Spec> {
        &self.deploy
    }

    fn deploy_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::deploy::Spec> {
        &mut self.deploy
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, rockspec::Error> {
        let lux_toml_dir = &self.internal.lux_toml_dir;
        let version = self
            .internal
            .version_template
            .try_generate(lux_toml_dir, None)
            .map_err(Error::from)?;
        let source = self
            .internal
            .source_template
            .try_generate(lux_toml_dir, &self.package, &version)
            .map_err(Error::from)?;
        Ok(self.render_rockspec(&version, Some(source.display_lua()))?)
    }
}

impl HasIntegrity for Local {
    async fn hash(&self) -> io::Result<Integrity> {
        match self.to_lua_rockspec() {
            Ok(lua_rockspec) => lua_rockspec.hash().await,
            Err(_) => self.internal.hash().await,
        }
    }
}
