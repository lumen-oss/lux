use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use miette::Diagnostic;
use target_lexicon::Triple;
use thiserror::Error;

use crate::{
    build::external_dependency::ExternalDependencyInfo, config::Config, fs, lua, rockspec,
};

use super::{c_link, CCExt};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("failed to compile intermediates from C files")]
    #[diagnostic(help(
        r#"check the compiler output above for missing headers or syntax errors.
ensure the C compiler has access to the required include paths."#
    ))]
    CompileIntermediates { source: cc::Error },
    #[error("error compiling C files (compilation failed):")]
    #[diagnostic(help(
        r#"check the compiler output above for details.
if no C compiler was found, run `lx debug toolchains` to verify your build tools."#
    ))]
    Compilation(#[from] cc::Error),
    #[error("error compiling C files (linking failed)")]
    #[diagnostic(forward(0))]
    Link(#[from] c_link::Error),
}

/// Compiles a set of C files into a single dynamic library and places them under `{target_dir}/{target_file}`.
/// # Panics
/// Panics if no parent or no filename can be determined for the target path.
#[derive(bon::Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Compiler<'a> {
    files: &'a [PathBuf],
    target_module: &'a rockspec::build::backend::Module,
    target_dir: &'a Path,
    lua: &'a lua::Installation,
    external_dependencies: &'a HashMap<String, ExternalDependencyInfo>,
    config: &'a Config,
}

impl<'a, State> CompilerBuilder<'a, State>
where
    State: compiler_builder::State + compiler_builder::IsComplete,
{
    #[tracing::instrument(name = "Compiling C files", level = "info", skip_all)]
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
        let target_file_name = target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        fs::sync::create_dir_all(target_parent_dir)?;

        let host = Triple::host();

        // See https://github.com/rust-lang/cc-rs/issues/594#issuecomment-2110551057

        let mut build = cc::Build::new();
        let intermediate_dir = fs::tempfile::tempdir()?;
        let build = build
            .cargo_output(compiler.config.verbose())
            .cargo_debug(compiler.config.verbose())
            .cargo_metadata(compiler.config.verbose())
            .cargo_warnings(compiler.config.verbose())
            .warnings(compiler.config.verbose())
            .files(compiler.files)
            .host(&host.to_string())
            .target(&host.to_string())
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
        // Suppress all warnings
        if cc_tool.is_like_msvc() {
            build.flag("-W0");
        } else {
            build.flag("-w");
        }
        for arg in compiler.lua.define_flags() {
            build.flag(&arg);
        }

        let objects = build
            .try_compile_objects(compiler.config)
            .map_err(|source| Error::CompileIntermediates { source })?;

        c_link::Linker::new()
            .cc_build(build)
            .lua(compiler.lua)
            .external_dependencies(compiler.external_dependencies)
            .target_file_name(&target_file_name)
            .libdir_args(Vec::new())
            .library_args(Vec::new())
            .target_module(compiler.target_module)
            .target_parent_dir(target_parent_dir)
            .objects(objects)
            .config(compiler.config)
            .link()
            .await?;
        Ok(())
    }
}
