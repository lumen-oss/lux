use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::fs;
use crate::lua_rockspec::TreesitterParserBuildSpec;
use crate::lua_version::LuaVersionUnset;
use crate::tree::InstallTree;
use miette::Diagnostic;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::info_span;
use tree_sitter_generate::GenerateError;

const DEFAULT_GENERATE_ABI_VERSION: usize = tree_sitter::LANGUAGE_VERSION;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum TreesitterBuildError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error("invalid TREE_SITTER_LANGUAGE_VERSION: {0}")]
    #[diagnostic(help(
        "check the value of the TREE_SITTER_LANGUAGE_VERSION environment variable."
    ))]
    ParseAbiVersion(#[from] ParseIntError),
    #[error("error generating tree-sitter grammar: {0}")]
    #[diagnostic(help("check the grammar source files for errors."))]
    Generate(#[from] GenerateError),
    #[error("error compiling the tree-sitter grammar: {0}")]
    #[diagnostic(help("run `lx debug toolchains` to check available build tools."))]
    TreesitterCompileError(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
}

impl BuildBackend for TreesitterParserBuildSpec {
    type Err = TreesitterBuildError;

    #[tracing::instrument(name = "treesitter_parser::run", skip_all, level = "debug")]
    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree,
    {
        let output_paths = args.output_paths;
        let build_dir = args.build_dir;
        let build_dir = self
            .location
            .map(|dir| build_dir.join(dir))
            .unwrap_or(build_dir.to_path_buf());
        if self.generate {
            let span = info_span!("Generating tree-sitter parser");
            let _enter = span.enter();
            let abi_version = match std::env::var("TREE_SITTER_LANGUAGE_VERSION") {
                Ok(v) => v.parse()?,
                Err(_) => DEFAULT_GENERATE_ABI_VERSION,
            };
            tracing::debug!("ABI version: {abi_version}");
            let out_path: Option<PathBuf> = None;
            let grammar_path: Option<PathBuf> = None;
            tree_sitter_generate::generate_parser_in_directory(
                &build_dir,
                out_path,
                grammar_path,
                abi_version,
                None,
                None,
                true,
                tree_sitter_generate::OptLevel::default(),
            )?;
        }
        if self.parser {
            build_parser(&build_dir, &output_paths.etc.join("parser"), &self.lang).await?;
        }

        let queries_dir = output_paths.etc.join("queries").join(&self.lang);
        install_queries(&build_dir, &queries_dir, &self.lang, self.queries).await?;

        Ok(BuildInfo::default())
    }
}

fn is_query_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "scm")
}

#[tracing::instrument(level = "trace")]
async fn install_inline_queries(
    queries_dir: &Path,
    queries: std::collections::HashMap<PathBuf, String>,
) -> Result<(), TreesitterBuildError> {
    fs::tokio::create_dir_all(queries_dir).await?;
    for (path, content) in queries {
        let dest = queries_dir.join(path);
        fs::tokio::write(&dest, content).await?;
    }
    Ok(())
}

#[tracing::instrument(level = "trace")]
async fn install_source_queries(
    source_queries_dir: &Path,
    queries_dir: &Path,
) -> Result<(), TreesitterBuildError> {
    fs::tokio::create_dir_all(queries_dir).await?;
    let mut entries = fs::tokio::read_dir(source_queries_dir).await?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| fs::FsError::ReadDir {
            path: source_queries_dir.to_path_buf(),
            source,
        })?
    {
        let path = entry.path();
        if let Some(filename) = path.file_name().filter(|_| is_query_file(&path)) {
            let dest = queries_dir.join(filename);
            fs::tokio::copy(&path, &dest).await?;
        }
    }
    Ok(())
}

#[tracing::instrument(level = "trace")]
async fn install_queries(
    build_dir: &Path,
    queries_dir: &Path,
    lang: &str,
    queries: std::collections::HashMap<PathBuf, String>,
) -> Result<(), TreesitterBuildError> {
    if !queries.is_empty() {
        install_inline_queries(queries_dir, queries).await
    } else {
        let source_queries_dir = build_dir.join("queries");
        let lang_queries_dir = source_queries_dir.join(lang);
        if source_queries_dir.is_dir() && !lang_queries_dir.is_dir() {
            install_source_queries(&source_queries_dir, queries_dir).await
        } else {
            Ok(())
        }
    }
}

#[tracing::instrument(level = "trace")]
async fn build_parser(
    build_dir: &Path,
    parser_dir: &Path,
    lang: &str,
) -> Result<(), TreesitterBuildError> {
    let span = info_span!("Compiling tree-sitter parser", language = lang);
    let _enter = span.enter();
    fs::tokio::create_dir_all(parser_dir).await?;
    let loader = tree_sitter_loader::Loader::with_parser_lib_path(build_dir.to_path_buf());
    let output_path = parser_dir.join(format!("{}.{}", lang, std::env::consts::DLL_EXTENSION));
    // HACK(vhyrro): `tree-sitter-loader` will only use a temp directory instead of a
    // lockfile if a `CROSS_RUNNER` env variable is set (why??). We should probably make a
    // PR fixing this with a flag. This should make-do for now: theoretically, this could
    // break since it's on an async thread, but in practice this will never be executed
    // many times during the lifetime of the `lx` binary.
    std::env::set_var("CROSS_RUNNER", "");
    loader
        .compile_parser_at_path(build_dir, output_path, &[])
        .map_err(|err| TreesitterBuildError::TreesitterCompileError(err.to_string()))
}
