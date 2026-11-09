use itertools::{Either, Itertools};
use miette::Diagnostic;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;
use walkdir::WalkDir;

use crate::{
    build::{
        backend::{BuildBackend, BuildInfo, RunBuildArgs},
        compile::{c_files, c_modules},
    },
    fs, rockspec,
    tree::{self, InstallTree},
};

#[cfg(not(feature = "luau-transpile"))]
use crate::lua;

use super::{compile, install};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCFiles(#[from] compile::c_files::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCModules(#[from] compile::c_modules::Error),
    #[error("failed to install binary '{file_name}'")]
    InstallBinary {
        file_name: String,
        #[diagnostic_source]
        source: install::Error,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error("module auto-detection failed")]
    #[diagnostic(transparent)]
    AutoDetectModules(#[from] AutoDetectModulesError),
    #[error("luau does not support native dependencies")]
    #[diagnostic(help(
        "the luau runtime cannot load native libraries. use pure-Luau packages or Lua"
    ))]
    LuauNativeDepsUnsupported,
    #[cfg(feature = "luau-transpile")]
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuauTranspile(#[from] super::luau::Error),
    #[cfg(not(feature = "luau-transpile"))]
    #[error("cannot install Luau module `{module}` on {lua_version}")]
    #[diagnostic(help(
        "this build of Lux was compiled without support for transpiling Luau to Lua"
    ))]
    LuauTranspileDisabled {
        module: String,
        lua_version: lua::Version,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Teal(#[from] compile::teal::Error),
}

impl BuildBackend for rockspec::build::backend::Builtin {
    type Err = Error;

    #[tracing::instrument(name = "builtin::run", skip_all, level = "debug")]
    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree + Sync,
    {
        let package = args.package;
        let lua = args.lua;
        let external_dependencies = args.external_dependencies;
        let config = args.config;
        let tree = args.tree;
        let layout = tree.layout_for(package);
        let build_dir = args.build_dir;
        let build_dependencies = args.build_dependencies;
        let dependency_layouts = args.dependency_layouts;

        let (detected_modules, teal_files) =
            autodetect_modules(build_dir, source_paths(build_dir, &self.modules))?;
        let modules = detected_modules
            .into_iter()
            .chain(self.modules)
            .collect::<HashMap<_, _>>();

        if lua.version.is_luau() {
            let has_native_modules = modules.values().any(|spec| match spec {
                rockspec::build::backend::ModuleSpec::SourcePath(source) => {
                    source.extension().is_some_and(|ext| ext == "c")
                }
                rockspec::build::backend::ModuleSpec::SourcePaths(_)
                | rockspec::build::backend::ModuleSpec::ModulePaths(_) => true,
            });
            if has_native_modules {
                return Err(Error::LuauNativeDepsUnsupported);
            }
        }

        for (destination_path, module_type) in modules.iter() {
            match module_type {
                rockspec::build::backend::ModuleSpec::SourcePath(source) => {
                    if source.extension().map(|ext| ext == "c").unwrap_or(false) {
                        let absolute_source_paths = vec![build_dir.join(source)];
                        c_files::Compiler::new()
                            .files(&absolute_source_paths)
                            .target_module(destination_path)
                            .target_dir(&layout.lib)
                            .lua(lua)
                            .external_dependencies(external_dependencies)
                            .config(config)
                            .compile()
                            .await?;
                    } else if source.extension().is_some_and(|ext| ext == "luau")
                        && !lua.version.is_luau()
                    {
                        #[cfg(feature = "luau-transpile")]
                        {
                            let absolute_source_path = build_dir.join(source);
                            super::luau::transpile_luau_to_lua_module(
                                &absolute_source_path,
                                destination_path,
                                &layout.src,
                            )
                            .await?;
                        }
                        #[cfg(not(feature = "luau-transpile"))]
                        return Err(Error::LuauTranspileDisabled {
                            module: destination_path.to_string(),
                            lua_version: lua.version.clone(),
                        });
                    } else {
                        let absolute_source_path = build_dir.join(source);
                        install::lua_module(&absolute_source_path, destination_path, &layout.src)?;
                    }
                }
                rockspec::build::backend::ModuleSpec::SourcePaths(files) => {
                    let absolute_source_paths: Vec<PathBuf> =
                        files.iter().map(|file| build_dir.join(file)).collect();
                    c_files::Compiler::new()
                        .files(&absolute_source_paths)
                        .target_module(destination_path)
                        .target_dir(&layout.lib)
                        .lua(lua)
                        .external_dependencies(external_dependencies)
                        .config(config)
                        .compile()
                        .await?
                }
                rockspec::build::backend::ModuleSpec::ModulePaths(data) => {
                    c_modules::Compiler::new()
                        .data(data)
                        .source_dir(build_dir)
                        .target_module(destination_path)
                        .target_dir(&layout.lib)
                        .lua(lua)
                        .external_dependencies(external_dependencies)
                        .dependency_layouts(dependency_layouts)
                        .config(config)
                        .compile()
                        .await?
                }
            }
        }

        if !teal_files.is_empty() {
            install::sources(build_dir, &layout.src, &teal_files)?;
            if build_dependencies
                .iter()
                .any(|dep| dep.name().to_string() == "tl")
            {
                compile::teal::compile(build_dir, &layout.src, &teal_files, lua, tree, config)
                    .await?;
            }
        }

        let mut binaries = Vec::new();
        for bin_script in install::detect_binaries(build_dir) {
            if let Some(target) = bin_script.file_name() {
                let file_name = target.to_string_lossy().to_string();
                let installed_bin_script =
                    install::binary(&bin_script, &file_name, tree, lua, args.deploy, config)
                        .await
                        .map_err(|err| Error::InstallBinary {
                            file_name: file_name.clone(),
                            source: err,
                        })?;
                if let Some(bin_script_file_name) = installed_bin_script.file_name() {
                    binaries.push(bin_script_file_name.into());
                }
            }
        }

        Ok(BuildInfo { binaries })
    }
}

fn source_paths(
    build_dir: &Path,
    modules: &HashMap<rockspec::build::backend::Module, rockspec::build::backend::ModuleSpec>,
) -> HashSet<PathBuf> {
    modules
        .values()
        .flat_map(|spec| match spec {
            rockspec::build::backend::ModuleSpec::SourcePath(path_buf) => vec![path_buf],
            rockspec::build::backend::ModuleSpec::SourcePaths(vec) => vec.iter().collect_vec(),
            rockspec::build::backend::ModuleSpec::ModulePaths(module_paths) => {
                module_paths.sources.iter().collect_vec()
            }
        })
        .map(|path| build_dir.join(path))
        .collect()
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum AutoDetectModulesError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    ParseLuaModule(#[from] rockspec::build::backend::ParseLuaModuleError),
}

#[tracing::instrument(level = "trace")]
pub(crate) fn autodetect_modules(
    build_dir: &Path,
    exclude: HashSet<PathBuf>,
) -> Result<
    (
        HashMap<rockspec::build::backend::Module, rockspec::build::backend::ModuleSpec>,
        Vec<PathBuf>,
    ),
    AutoDetectModulesError,
> {
    let (modules, teal_files): (Vec<_>, Vec<_>) = WalkDir::new(build_dir.join("src"))
        .into_iter()
        .chain(WalkDir::new(build_dir.join("lua")))
        .chain(WalkDir::new(build_dir.join("lib")))
        .filter_map(|file| {
            file.ok().and_then(|file| {
                let is_source = PathBuf::from(file.file_name())
                    .extension()
                    .is_some_and(|ext| matches!(ext.to_str(), Some("lua" | "luau" | "tl")));
                if is_source
                    && file.file_type().is_file()
                    && !exclude.contains(&file.clone().into_path())
                {
                    Some(file)
                } else {
                    None
                }
            })
        })
        .map(|file| {
            let diff: PathBuf = unsafe {
                pathdiff::diff_paths(build_dir.join(file.clone().into_path()), build_dir)
                    .unwrap_unchecked()
            };
            if diff.extension().is_some_and(|ext| ext == "tl") {
                return Ok::<_, AutoDetectModulesError>(Either::Right(diff));
            }

            // NOTE(vhyrro): You may ask why we convert all paths to Lua module paths
            // just to convert them back later in the `run()` stage.
            //
            // The rockspec requires the format to be like this, and representing our
            // data in this form allows us to respect any overrides made by the user (which follow
            // the `module.name` format, not our internal one).
            let mut pathbuf = diff.components().skip(1).collect::<PathBuf>();
            let lua_module = if pathbuf
                .parent()
                .is_none_or(|parent| parent.as_os_str().is_empty())
            {
                pathbuf.set_extension("");
                rockspec::build::backend::Module::from_pathbuf(pathbuf)?
            } else {
                let mut lua_module = rockspec::build::backend::Module::from_pathbuf(pathbuf)?;
                // NOTE(mrcjkb): `build::backend::Module` does not parse as "<module>.init" from files named "init.lua"
                // To make sure we don't change the file structure when installing, we append it here.
                if matches!(
                    file.file_name().to_string_lossy().as_bytes(),
                    b"init.lua" | b"init.luau"
                ) {
                    unsafe {
                        lua_module = lua_module.join(
                            &rockspec::build::backend::Module::from_str("init").unwrap_unchecked(),
                        );
                    }
                }
                lua_module
            };
            Ok::<_, AutoDetectModulesError>(Either::Left((
                lua_module,
                rockspec::build::backend::ModuleSpec::SourcePath(diff),
            )))
        })
        .try_collect::<_, Vec<_>, _>()?
        .into_iter()
        .partition_map(|detected| detected);
    Ok((modules.into_iter().collect(), teal_files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"return {}").unwrap();
    }

    #[tokio::test]
    async fn test_auto_detect_modules() {
        let build_dir = tempfile::tempdir().unwrap();
        let root = build_dir.path();
        write_file(&root.join("src").join("foo.luau"));
        write_file(&root.join("src").join("bar").join("baz.luau"));
        write_file(&root.join("src").join("baz").join("init.luau"));
        write_file(&root.join("lua").join("legacy.lua"));
        write_file(&root.join("src").join("teal").join("mod.tl"));
        write_file(&root.join("src").join("teal.d.tl"));

        let (modules, teal_files) = autodetect_modules(root, Default::default()).unwrap();

        let names = modules.keys().map(|m| m.to_string()).collect_vec();
        assert!(names.contains(&"foo".to_string()), "{names:?}");
        assert!(names.contains(&"bar.baz".to_string()), "{names:?}");
        assert!(names.contains(&"baz.init".to_string()), "{names:?}");
        assert!(names.contains(&"legacy".to_string()), "{names:?}");

        let foo_spec = &modules[&"foo".parse().unwrap()];
        match foo_spec {
            rockspec::build::backend::ModuleSpec::SourcePath(path) => {
                assert_eq!(path.extension().unwrap(), "luau");
            }
            _ => panic!("expected SourcePath for foo.luau"),
        }

        let mut teal_files = teal_files
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect_vec();
        teal_files.sort();
        assert_eq!(teal_files, vec!["src/teal.d.tl", "src/teal/mod.tl"]);
    }
}
