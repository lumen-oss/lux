use std::{collections::HashMap, path::Path};

use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use target_lexicon::Triple;
use thiserror::Error;

use crate::{
    build::{dependency, external_dependency::ExternalDependencyInfo},
    config::Config,
    fs, lua, rockspec,
    variables::{self, Environment},
};

use super::{c_link, CCExt};

// TODO: (#261): special cases for mingw/cygwin?

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("failed to compile intermediates from C modules")]
    #[diagnostic(help(
        r#"check the compiler output above for missing headers or syntax errors.
ensure the C compiler has access to the required include paths."#
    ))]
    CompileIntermediates(#[source] cc::Error),
    #[error("error compiling C modules (compilation failed)")]
    #[diagnostic(help(
        r#"check the compiler output above for details.
if no C compiler was found, run `lx debug toolchains` to verify your build tools."#
    ))]
    Compilation(#[from] cc::Error),
    #[error("error compiling C modules (linking failed)")]
    #[diagnostic(forward(0))]
    Link(#[from] c_link::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    VariableSubstitution(#[from] variables::Error),
}

/// Compiles a set of C files (with extra metadata) to a given destination.
/// # Panics
/// Panics if no filename for the target path can be determined.
#[derive(bon::Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Compiler<'a> {
    data: &'a rockspec::build::backend::ModulePaths,
    source_dir: &'a Path,
    target_module: &'a rockspec::build::backend::Module,
    target_dir: &'a Path,
    lua: &'a lua::Installation,
    external_dependencies: &'a HashMap<String, ExternalDependencyInfo>,
    dependency_layouts: &'a dependency::Layouts,
    config: &'a Config,
}

impl<'a, State> CompilerBuilder<'a, State>
where
    State: compiler_builder::State + compiler_builder::IsComplete,
{
    #[tracing::instrument(name = "Compiling C modules", level = "info", skip_all)]
    pub(crate) async fn compile(self) -> Result<(), Error> {
        let compiler = self._build();
        let target = compiler
            .target_dir
            .join(compiler.target_module.to_lib_path());

        let target_parent_dir = target.parent().unwrap_or_else(|| {
            unreachable!(
                "couldn't determine build target parent directory:\n{}",
                target.display()
            )
        });

        fs::tokio::create_dir_all(target_parent_dir).await?;

        let host = Triple::host();

        let mut build = cc::Build::new();
        let source_files = compiler
            .data
            .sources
            .iter()
            .map(|dir| {
                variables::substitute(
                    &[
                        compiler.lua,
                        compiler.external_dependencies,
                        compiler.dependency_layouts,
                        &Environment {},
                        compiler.config,
                    ],
                    &dir.to_slash_lossy(),
                )
            })
            .map(|dir| dir.map(|dir| compiler.source_dir.join(dir)))
            .try_collect::<_, Vec<_>, _>()?;
        let include_dirs = compiler
            .data
            .incdirs
            .iter()
            .map(|dir| {
                variables::substitute(
                    &[
                        compiler.lua,
                        compiler.external_dependencies,
                        compiler.dependency_layouts,
                        &Environment {},
                        compiler.config,
                    ],
                    &dir.to_slash_lossy(),
                )
            })
            .map(|dir| dir.map(|dir| compiler.source_dir.join(dir)))
            .chain(
                compiler
                    .external_dependencies
                    .values()
                    .filter_map(|dep| dep.include_dir.clone())
                    .map(Result::Ok),
            )
            .try_collect::<_, Vec<_>, _>()?
            .into_iter()
            .unique() // Some rocks specify the include dirs via variable substitution.
            .collect_vec();

        let intermediate_dir = fs::tempfile::tempdir()?;
        let build = build
            .cargo_output(compiler.config.verbose())
            .cargo_debug(compiler.config.verbose())
            .cargo_metadata(compiler.config.verbose())
            .cargo_warnings(compiler.config.verbose())
            .warnings(compiler.config.verbose())
            .files(source_files)
            .host(&host.to_string())
            .target(&host.to_string())
            .includes(&include_dirs)
            .includes(compiler.lua.includes())
            .includes(
                compiler
                    .external_dependencies
                    .values()
                    .filter_map(|dep| dep.include_dir.as_ref()),
            )
            .opt_level(compiler.config.build_profile().opt_level())
            .out_dir(intermediate_dir);

        let cc_tool = build.try_get_compiler()?;
        let is_msvc = cc_tool.is_like_msvc();
        // Suppress all warnings
        if is_msvc {
            build.flag("-W0");
        } else {
            build.flag("-w");
        }
        for arg in compiler.lua.define_flags() {
            build.flag(&arg);
        }

        // `cc::Build` has no `defines()` function, so we manually feed in the
        // definitions in a verbose loop
        for (name, value) in &compiler.data.defines {
            build.define(name, value.as_deref());
        }

        let target_file_name = target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // See https://github.com/rust-lang/cc-rs/issues/594#issuecomment-2110551057
        let objects = build
            .try_compile_objects(compiler.config)
            .map_err(Error::CompileIntermediates)?;

        let libdir_args = compiler
            .data
            .libdirs
            .iter()
            .map(|dir| {
                variables::substitute(
                    &[
                        compiler.lua,
                        compiler.external_dependencies,
                        compiler.dependency_layouts,
                        &Environment {},
                        compiler.config,
                    ],
                    &dir.to_slash_lossy(),
                )
            })
            .map(|libdir| {
                libdir.map(|libdir| {
                    if is_msvc {
                        format!("/LIBPATH:{}", compiler.source_dir.join(libdir).display())
                    } else {
                        format!("-L{}", compiler.source_dir.join(libdir).display())
                    }
                })
            })
            .try_collect::<_, Vec<_>, _>()?;

        let library_args = compiler
            .data
            .libraries
            .iter()
            .map(|dir| {
                variables::substitute(
                    &[
                        compiler.lua,
                        compiler.external_dependencies,
                        compiler.dependency_layouts,
                        &Environment {},
                        compiler.config,
                    ],
                    &dir.to_slash_lossy(),
                )
            })
            .map(|library| {
                library.map(|library| {
                    if is_msvc {
                        format!("{}.lib", library)
                    } else {
                        format!("-l{}", library)
                    }
                })
            })
            .try_collect::<_, Vec<_>, _>()?;

        c_link::Linker::new()
            .cc_build(build)
            .lua(compiler.lua)
            .external_dependencies(compiler.external_dependencies)
            .target_file_name(&target_file_name)
            .libdir_args(libdir_args)
            .library_args(library_args)
            .target_module(compiler.target_module)
            .target_parent_dir(target_parent_dir)
            .objects(objects)
            .config(compiler.config)
            .link()
            .await?;
        Ok(())
    }
}
