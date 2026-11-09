use std::collections::HashMap;
use std::io;

use itertools::Itertools;
use ssri::Integrity;

use crate::hash::HasIntegrity;
use crate::package;
use crate::package::{BuildDependencies, Dependencies};
use crate::rockspec::{self, display::AsLuaKV, Rockspec};

use super::local::Local;
use super::{build_spec_with_autodetected_modules, validate_generated_lua, Error};

/// The `lux.toml`, after being validated and prepared for upload
#[derive(Debug)]
pub struct Remote {
    pub(super) local: Local,
    pub(super) source: rockspec::PerPlatform<rockspec::source::Remote>,
}

impl Remote {
    pub fn to_lua_rockspec(&self) -> Result<rockspec::lua::Remote, rockspec::Error> {
        Ok(rockspec::lua::Remote::new(
            &self.to_lua_remote_rockspec_string()?,
        )?)
    }
}

impl Rockspec for Remote {
    fn package(&self) -> &package::Name {
        self.local.package()
    }

    fn version(&self) -> &package::Version {
        self.local.version()
    }

    fn description(&self) -> &rockspec::description::Spec {
        self.local.description()
    }

    fn supported_platforms(&self) -> &rockspec::platform::Support {
        self.local.supported_platforms()
    }

    fn lua(&self) -> &package::VersionReq {
        self.local.lua()
    }

    fn dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.dependencies()
    }

    fn build_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.build_dependencies()
    }

    fn external_dependencies(
        &self,
    ) -> &rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>> {
        self.local.external_dependencies()
    }

    fn test_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.test_dependencies()
    }

    fn build(&self) -> &rockspec::PerPlatform<rockspec::build::Spec> {
        self.local.build()
    }

    fn test(&self) -> &rockspec::PerPlatform<rockspec::test::Spec> {
        self.local.test()
    }

    fn build_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::build::Spec> {
        self.local.build_mut()
    }

    fn test_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::test::Spec> {
        self.local.test_mut()
    }

    fn format(&self) -> &Option<rockspec::Format> {
        self.local.format()
    }

    fn source(&self) -> &rockspec::PerPlatform<rockspec::source::Remote> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::source::Remote> {
        &mut self.source
    }

    fn deploy(&self) -> &rockspec::PerPlatform<rockspec::deploy::Spec> {
        self.local.deploy()
    }

    fn deploy_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::deploy::Spec> {
        self.local.deploy_mut()
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, rockspec::Error> {
        let lux_toml_dir = &self.local.internal.lux_toml_dir;
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.local.generated_rockspec_format(),
            self.local.package,
            self.version()
        );

        let mut template = Vec::new();

        if self.local.description != rockspec::description::Spec::default() {
            template.push(self.local.description.display_lua());
        }

        if self.local.supported_platforms != rockspec::platform::Support::default() {
            template.push(self.local.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.local.internal.dependencies.clone().unwrap_or_default();
            if !self.local.lua.is_luau() {
                dependencies.insert(
                    0,
                    package::Req {
                        name: "lua".into(),
                        version_req: self.local.lua.clone(),
                    }
                    .into(),
                );
            }
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
                template.push(
                    rockspec::external_dependency::Dependencies(external_dependencies)
                        .display_lua(),
                );
            }
            _ => {}
        }

        let source = self
            .local
            .internal
            .source_template
            .try_generate(lux_toml_dir, &self.local.internal.package, self.version())
            .map_err(Error::from)?;
        template.push(source.display_lua());

        if let Some(ref deploy) = self.local.internal.deploy {
            template.push(deploy.display_lua());
        }

        template.push(
            build_spec_with_autodetected_modules(&self.local.internal.build, lux_toml_dir)
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

impl HasIntegrity for Remote {
    async fn hash(&self) -> io::Result<Integrity> {
        self.to_lua_rockspec()
            .map_err(|err| {
                io::Error::other(format!(
                    "unable to convert remote project to Lua rockspec:\n{}",
                    err
                ))
            })?
            .hash()
            .await
    }
}
