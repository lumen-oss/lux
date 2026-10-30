use std::{collections::HashMap, path::Path};

use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use target_lexicon::Triple;
use thiserror::Error;

use crate::{
    build::external_dependency::ExternalDependencyInfo,
    config::Config,
    fs, lua, rockspec,
    variables::{self, Environment, VariableSubstitutionError},
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
    VariableSubstitution(#[from] VariableSubstitutionError),
}

/// Compiles a set of C files (with extra metadata) to a given destination.
/// # Panics
/// Panics if no filename for the target path can be determined.
#[tracing::instrument(
    name = "Compiling C modules",
    level = "info",
    skip_all,
    fields(profile = config.build_profile().to_string()),
)]
pub(crate) async fn compile(
    data: &rockspec::build::backend::ModulePaths,
    source_dir: &Path,
    target_module: &rockspec::build::backend::Module,
    target_dir: &Path,
    lua: &lua::Installation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    config: &Config,
) -> Result<(), Error> {
    let target = target_dir.join(target_module.to_lib_path());

    let target_parent_dir = target.parent().unwrap_or_else(|| {
        unreachable!(
            "couldn't determine build target parent directory:\n{}",
            target.display()
        )
    });

    fs::tokio::create_dir_all(target_parent_dir).await?;

    let host = Triple::host();

    let mut build = cc::Build::new();
    let source_files = data
        .sources
        .iter()
        .map(|dir| {
            variables::substitute(
                &[lua, external_dependencies, &Environment {}, config],
                &dir.to_slash_lossy(),
            )
        })
        .map(|dir| dir.map(|dir| source_dir.join(dir)))
        .try_collect::<_, Vec<_>, _>()?;
    let include_dirs = data
        .incdirs
        .iter()
        .map(|dir| {
            variables::substitute(
                &[lua, external_dependencies, &Environment {}, config],
                &dir.to_slash_lossy(),
            )
        })
        .map(|dir| dir.map(|dir| source_dir.join(dir)))
        .chain(
            external_dependencies
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
        .cargo_output(config.verbose())
        .cargo_debug(config.verbose())
        .cargo_metadata(config.verbose())
        .cargo_warnings(config.verbose())
        .warnings(config.verbose())
        .files(source_files)
        .host(&host.to_string())
        .target(&host.to_string())
        .includes(&include_dirs)
        .includes(lua.includes())
        .includes(
            external_dependencies
                .values()
                .filter_map(|dep| dep.include_dir.as_ref()),
        )
        .opt_level(config.build_profile().opt_level())
        .out_dir(intermediate_dir);

    let compiler = build.try_get_compiler()?;
    let is_msvc = compiler.is_like_msvc();
    // Suppress all warnings
    if is_msvc {
        build.flag("-W0");
    } else {
        build.flag("-w");
    }
    for arg in lua.define_flags() {
        build.flag(&arg);
    }

    // `cc::Build` has no `defines()` function, so we manually feed in the
    // definitions in a verbose loop
    for (name, value) in &data.defines {
        build.define(name, value.as_deref());
    }

    let target_file_name = target
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    // See https://github.com/rust-lang/cc-rs/issues/594#issuecomment-2110551057
    let objects = build
        .try_compile_objects(config)
        .map_err(Error::CompileIntermediates)?;

    let libdir_args = data
        .libdirs
        .iter()
        .map(|dir| {
            variables::substitute(
                &[lua, external_dependencies, &Environment {}, config],
                &dir.to_slash_lossy(),
            )
        })
        .map(|libdir| {
            libdir.map(|libdir| {
                if is_msvc {
                    format!("/LIBPATH:{}", source_dir.join(libdir).display())
                } else {
                    format!("-L{}", source_dir.join(libdir).display())
                }
            })
        })
        .try_collect::<_, Vec<_>, _>()?;

    let library_args = data
        .libraries
        .iter()
        .map(|dir| {
            variables::substitute(
                &[lua, external_dependencies, &Environment {}, config],
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

    c_link::artifacts(
        build,
        lua,
        external_dependencies,
        &target_file_name,
        libdir_args,
        library_args,
        target_module,
        target_parent_dir,
        objects,
        config,
    )
    .await?;
    Ok(())
}
