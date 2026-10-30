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
#[tracing::instrument(
    name = "Compiling C files",
    level = "info",
    skip_all,
    fields(profile = config.build_profile().to_string()),
)]
pub(crate) async fn compile(
    files: &Vec<PathBuf>,
    target_module: &rockspec::build::Module,
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
        .cargo_output(config.verbose())
        .cargo_debug(config.verbose())
        .cargo_metadata(config.verbose())
        .cargo_warnings(config.verbose())
        .warnings(config.verbose())
        .files(files)
        .host(&host.to_string())
        .target(&host.to_string())
        .includes(lua.includes())
        .includes(
            external_dependencies
                .values()
                .filter_map(|dep| dep.include_dir.as_ref()),
        )
        .opt_level(config.build_profile().opt_level())
        .out_dir(intermediate_dir);

    let compiler = build.try_get_compiler()?;
    // Suppress all warnings
    if compiler.is_like_msvc() {
        build.flag("-W0");
    } else {
        build.flag("-w");
    }
    for arg in lua.define_flags() {
        build.flag(&arg);
    }

    let objects = build
        .try_compile_objects(config)
        .map_err(|source| Error::CompileIntermediates { source })?;

    c_link::artifacts(
        build,
        lua,
        external_dependencies,
        &target_file_name,
        Vec::new(),
        Vec::new(),
        target_module,
        target_parent_dir,
        objects,
        config,
    )
    .await?;
    Ok(())
}
