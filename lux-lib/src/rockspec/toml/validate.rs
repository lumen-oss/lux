use itertools::Itertools;
use std::path::PathBuf;

use crate::package;
use crate::rockspec;

use super::{Error, Local, Partial, Remote};
use crate::rockspec::lua_dependency;

impl Partial {
    /// Convert the `Partial` struct into a `Local` struct, making
    /// it ready to be used for building a project.
    pub fn into_local(&self) -> Result<Local, Error> {
        let project_toml = self.clone();

        // Disallow `lua` to be part of the `dependencies` field
        if project_toml
            .dependencies
            .as_ref()
            .is_some_and(|deps| deps.iter().any(|dep| dep.name() == &"lua".into()))
        {
            return Err(Error::DependenciesContainLua);
        }

        let get_duplicates = |dependencies: &Option<Vec<lua_dependency::Spec>>| {
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
            return Err(Error::DuplicateDependencies(package::List::new(
                duplicate_dependencies,
            )));
        }
        let duplicate_test_dependencies = get_duplicates(&self.test_dependencies);
        if !duplicate_test_dependencies.is_empty() {
            return Err(Error::DuplicateTestDependencies(package::List::new(
                duplicate_test_dependencies,
            )));
        }
        let duplicate_build_dependencies = get_duplicates(&self.build_dependencies);
        if !duplicate_build_dependencies.is_empty() {
            return Err(Error::DuplicateBuildDependencies(package::List::new(
                duplicate_build_dependencies,
            )));
        }

        let validated = Local {
            internal: project_toml.clone(),

            package: project_toml.package,
            version: project_toml
                .version_template
                .try_generate(&self.lux_toml_dir, None)
                .unwrap_or(package::Version::default_dev_version()),
            lua: project_toml.lua.ok_or(Error::NoLuaVersion)?,
            description: project_toml.description.unwrap_or_default(),
            run: project_toml.run.map(rockspec::PerPlatform::new),
            supported_platforms: rockspec::platform::Support::parse(
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
            dependencies: rockspec::PerPlatform::new(
                project_toml
                    .dependencies
                    .unwrap_or_default()
                    .into_iter()
                    .map(|dep| self.resolve_lua_dependency_spec(dep))
                    .collect_vec(),
            ),
            test_dependencies: rockspec::PerPlatform::new(
                project_toml
                    .test_dependencies
                    .unwrap_or_default()
                    .into_iter()
                    .map(|dep| self.resolve_lua_dependency_spec(dep))
                    .collect_vec(),
            ),
            build_dependencies: rockspec::PerPlatform::new(
                project_toml
                    .build_dependencies
                    .unwrap_or_default()
                    .into_iter()
                    .map(|dep| self.resolve_lua_dependency_spec(dep))
                    .collect_vec(),
            ),
            external_dependencies: rockspec::PerPlatform::new(
                project_toml.external_dependencies.unwrap_or_default(),
            ),
            test: rockspec::PerPlatform::new(rockspec::test::Spec::try_from(
                project_toml.test.clone().unwrap_or_default(),
            )?),
            build: rockspec::PerPlatform::new(rockspec::build::Spec::from_internal_spec(
                project_toml.build.clone(),
            )?),
            deploy: rockspec::PerPlatform::new(project_toml.deploy.clone().unwrap_or_default()),
            rockspec_format: project_toml.rockspec_format.clone(),

            source: rockspec::PerPlatform::new(rockspec::source::Remote::new(
                rockspec::source::Local::default(),
                rockspec::source::Spec::File(self.project.root.to_path_buf()),
            )),
        };

        let rockspec_file_name = format!("{}-{}.rockspec", validated.package, validated.version);

        if validated
            .build
            .default
            .copy_directories
            .contains(&PathBuf::from(&rockspec_file_name))
        {
            return Err(Error::CopyDirectoriesContainRockspecName(None));
        }

        for (platform, build_override) in &validated.build.per_platform {
            if build_override
                .copy_directories
                .contains(&PathBuf::from(&rockspec_file_name))
            {
                return Err(Error::CopyDirectoriesContainRockspecName(Some(
                    platform.to_string(),
                )));
            }
        }

        Ok(validated)
    }

    /// Convert the `Partial` struct into a `Remote` struct, making
    /// it ready to be serialized into a rockspec.
    /// A source must be provided for the rockspec to be valid.
    pub fn into_remote(&self, specrev: Option<package::version::SpecRev>) -> Result<Remote, Error> {
        self.into_remote_with_version(None, specrev)
    }

    /// Like [`Self::into_remote`] but generates the rockspec for a specified
    /// target version.
    pub fn into_remote_with_version(
        &self,
        version: Option<package::Version>,
        specrev: Option<package::version::SpecRev>,
    ) -> Result<Remote, Error> {
        let version = match version {
            Some(version) => version,
            None => self
                .version_template
                .try_generate(&self.lux_toml_dir, specrev)?,
        };
        let source =
            self.source_template
                .try_generate(&self.lux_toml_dir, &self.package, &version)?;
        let source = rockspec::PerPlatform::new(
            rockspec::source::Remote::try_from(source).map_err(Error::RockSource)?,
        );
        let mut local = self.into_local()?;
        local.version = version;

        let validated = Remote { source, local };

        Ok(validated)
    }
}
