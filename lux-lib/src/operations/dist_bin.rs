use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

use bon::Builder;
use itertools::Itertools;
use thiserror::Error;
use walkdir::WalkDir;

use crate::{
    build::utils::c_dylib_extension,
    config::Config,
    lua_installation::{LuaInstallation, LuaInstallationError},
    lua_rockspec::LuaModule,
    operations::{InstallProject, InstallProjectError},
    project::{project_toml::LocalProjectTomlValidationError, Project},
    rockspec::Rockspec,
    tree::{InstallTree, TreeError},
};

/// Compile a Lux project and all its dependencies into a single
/// static binary that does not require a Lua installation.
///
/// Based on [luastatic](https://github.com/ers35/luastatic)
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct DistProjectBin<'a, T>
where
    T: InstallTree,
{
    /// The project to compile.
    project: &'a Project,

    config: &'a Config,

    /// Tree in which to install the project before compiling.
    tree: &'a T,

    /// Destination path for the compiled binary.
    /// Defaults to `<cwd>/<package>[.exe]`.
    output: Option<PathBuf>,
}

use miette::Diagnostic;
#[derive(Error, Debug, Diagnostic)]
pub enum DistProjectBinError {
    #[error("error installing project:\n{0}")]
    #[diagnostic(forward(0))]
    InstallProject(#[from] InstallProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[cfg(not(target_os = "linux"))]
    #[error(
        r#"Lua binary libraries are only linkable on Linux.
Cannot link the following binaries:
{0}"#
    )]
    CannotLinkBinaryLibs(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    CC(#[from] cc::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("C compilation failed (exit {status}):\nstdout: {stdout}\nstderr: {stderr}")]
    CompilationFailed {
        status: std::process::ExitStatus,
        stdout: String,
        stderr: String,
    },
}

impl<T, State> DistProjectBinBuilder<'_, T, State>
where
    T: InstallTree + Sync + Send + Clone + 'static,
    State: dist_project_bin_builder::State + dist_project_bin_builder::IsComplete,
{
    pub async fn compile(self) -> Result<PathBuf, DistProjectBinError> {
        do_dist_project_bin(self._build()).await
    }
}

/// Files collected from an installed tree for binary compilation.
#[derive(Debug)]
struct InstalledFiles {
    /// Lua source files (.lua) to embed.
    src: Vec<(LuaModule, PathBuf)>,
    /// Native Lua modules (.so/.dll/.dylib) to link.
    lib: Vec<PathBuf>,
}

async fn do_dist_project_bin<T>(args: DistProjectBin<'_, T>) -> Result<PathBuf, DistProjectBinError>
where
    T: InstallTree + Sync + Send + Clone + 'static,
{

    let package = InstallProject::new()
        .project(args.project)
        .config(args.config)
        .tree(args.tree)
        .build()
        .await?;

    let files = collect_installed_files(args.tree)?;

    let project_toml = args.project.toml().into_local()?;
    let pkg_name = project_toml.package().to_string();

    let entrypoint_module = project_toml
        .run()
        .and_then(|r| r.current_platform().args.as_ref())
        .map(|args| args.first().as_str())
        .map(entrypoint_stem)
        .unwrap_or_else(|| pkg_name.clone());

    let layout = args.tree.installed_rock_layout(&package)?;

    let lua = LuaInstallation::new_from_config(args.config).await?;

    let output = args.output.unwrap_or_else(|| {
        let mut p = PathBuf::from(&pkg_name);
        if cfg!(target_env = "msvc") {
            p.set_extension("exe");
        }
        p
    });

    let lib_root = layout.lib.clone();
    let c_src = generate_c_source(&entrypoint_module, &files, &lib_root).await?;

    let work_dir = tempfile::tempdir()?;
    let c_path = work_dir.path().join(format!("{pkg_name}.static.c"));
    tokio::fs::write(&c_path, &c_src).await?;

    compile_binary(&c_path, &output, &lua, &files.lib, &work_dir, args.config).await?;

    Ok(output)
}

#[allow(clippy::result_large_err)]
fn collect_installed_files(tree: &impl InstallTree) -> Result<InstalledFiles, DistProjectBinError> {
    let mut lua_sources = Vec::new();
    let mut native_modules = Vec::new();
    let c_dylib_ext = c_dylib_extension();

    for package in tree.list()?.values().flatten() {
        let layout = tree.installed_rock_layout(package)?;

        if layout.src.is_dir() {
            let src_canonical = layout.src.canonicalize().unwrap_or(layout.src.clone());
            for path in WalkDir::new(&src_canonical)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "lua"))
            {
                let rel = path
                    .strip_prefix(&src_canonical)
                    .unwrap_or(&path)
                    .with_extension("");
                if let Ok(module) = LuaModule::from_pathbuf(rel) {
                    lua_sources.push((module, path));
                }
            }
        }

        if layout.lib.is_dir() {
            let lib_canononical = layout.lib.canonicalize().unwrap_or(layout.lib.clone());
            for path in WalkDir::new(&lib_canononical)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == c_dylib_ext))
            {
                native_modules.push(path);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    if !native_modules.is_empty() {
        return Err(DistProjectBinError::CannotLinkBinaryLibs(
            native_modules
                .iter()
                .unique()
                .map(|p| p.to_string_lossy())
                .join("\n")));
    }

    // NOTE: A `FlatDistTree` can produce duplicates, as all modules share the same `src`.
    Ok(InstalledFiles {
        src: lua_sources.into_iter().unique().collect(),
        lib: native_modules.into_iter().unique().collect(),
    })
}

/// Derive a module stem from a run-spec arg like `"src/main.lua"` -> `"main"`.
fn entrypoint_stem(arg: &str) -> String {
    PathBuf::from(arg)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| arg.to_owned())
}

/// Derive the Lua module name from an installed lib path.
/// e.g. `<tree>/lib/foo/bar.so` -> `("foo.bar", "foo_bar")`.
fn module_names(lib_root: &Path, path: &Path) -> (String, String) {
    let rel = path.strip_prefix(lib_root).unwrap_or(path);
    let dotpath = rel
        .with_extension("")
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, ".");
    let underscore = dotpath.replace(['.', '-'], "_");
    (dotpath, underscore)
}

/// Generate a C source file embedding all Lua sources.
async fn generate_c_source(
    entrypoint_module: &str,
    files: &InstalledFiles,
    lib_root: &Path) -> Result<String, io::Error> {
    let mut out = String::from(C_PREAMBLE);

    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for path in &files.lib {
        let (_, underscore) = module_names(lib_root, path);
        out.push_str(&format!("int luaopen_{underscore}(lua_State *L);\n"));
    }
    out.push_str("#ifdef __cplusplus\n}\n#endif\n\n");

    for (i, (_, path)) in files.src.iter().enumerate() {
        let bytes = tokio::fs::read(path).await.map_err(|err| {
            io::Error::other(format!("unable to read '{}': {err}", path.display()))
        })?;
        let hex = bytes_to_hex(&bytes);
        out.push_str(&format!(
            "static const unsigned char lua_src_{i}[] = {{{hex}}};\n"
        ));
    }

    let loader_with_entrypoint = format!(
        "{LUA_LOADER_SOURCE}local func = lua_loader(\"{entrypoint_module}\")\n\
         if type(func) == \"function\" then\n\
         \tfunc(unpack(arg))\n\
         else\n\
         \terror(func, 0)\n\
         end\n"
    );
    let loader_hex = bytes_to_hex(loader_with_entrypoint.as_bytes());
    out.push_str(&format!(
        "static const unsigned char lua_loader_program[] = {{{loader_hex}}};\n\n"
    ));

    out.push_str("int main(int argc, char *argv[]) {\n");
    out.push_str("  lua_State *L = luaL_newstate();\n");
    out.push_str("  luaL_openlibs(L);\n");
    out.push_str("  createargtable(L, argv, argc, 0);\n\n");

    out.push_str(&format!(
        "  if (luaL_loadbuffer(L, (const char*)lua_loader_program, sizeof(lua_loader_program), \"{entrypoint_module}\") != LUA_OK) {{\n"
    ));
    out.push_str("    fprintf(stderr, \"luaL_loadbuffer: %s\\n\", lua_tostring(L, -1));\n");
    out.push_str("    lua_close(L); return 1;\n  }\n\n");

    out.push_str("  /* lua_bundle */\n  lua_newtable(L);\n");

    for (i, (module, _)) in files.src.iter().enumerate() {
        out.push_str(&format!(
            "  lua_pushlstring(L, (const char*)lua_src_{i}, sizeof(lua_src_{i}));\n"
        ));
        out.push_str(&format!("  lua_setfield(L, -2, \"{module}\");\n"));
    }

    for path in &files.lib {
        let (dotpath, underscore) = module_names(lib_root, path);
        out.push_str(&format!("  lua_pushcfunction(L, luaopen_{underscore});\n"));
        out.push_str(&format!("  lua_setfield(L, -2, \"{dotpath}\");\n"));
    }

    out.push_str("\n  if (docall(L, 1, LUA_MULTRET)) {\n");
    out.push_str("    const char *msg = lua_tostring(L, 1);\n");
    out.push_str("    if (msg) fprintf(stderr, \"%s\\n\", msg);\n");
    out.push_str("    lua_close(L); return 1;\n  }\n");
    out.push_str("  lua_close(L);\n  return 0;\n}\n");

    Ok(out)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect::<Vec<_>>()
        .join(", ")
}

async fn compile_binary(
    c_path: &Path,
    output: &Path,
    lua: &LuaInstallation,
    native_modules: &[PathBuf],
    work_dir: &tempfile::TempDir,
    config: &Config) -> Result<(), DistProjectBinError> {
    let mut build = cc::Build::new();
    let host = target_lexicon::Triple::host().to_string();

    let intermediate_dir = tempfile::tempdir()?;
    build
        .cargo_output(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .warnings(config.verbose())
        .host(&host)
        .target(&host)
        .opt_level(3)
        .out_dir(&intermediate_dir);

    let compiler = build.try_get_compiler()?;

    let is_msvc = compiler.is_like_msvc();
    // Suppress all warnings
    if is_msvc {
        build.flag("-W0");
    } else {
        build.flag("-w");
    }

    let mut cmd: tokio::process::Command = compiler.to_command().into();
    cmd.current_dir(work_dir.path());
    cmd.arg(c_path);

    for include in lua.includes() {
        cmd.arg(format!("-I{}", include.display()));
    }
    cmd.args(native_modules);
    cmd.arg("-o").arg(output);
    cmd.args(lua.lib_link_args(&compiler));

    #[cfg(not(target_env = "msvc"))]
    {
        cmd.arg("-rdynamic");
        cmd.arg("-lm");
        // Link with libdl because liblua was built with support loading
        // shared objects and the operating system depends on it.
        #[cfg(target_family = "unix")]
        cmd.arg("-ldl");
    }

    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !out.status.success() {
        return Err(DistProjectBinError::CompilationFailed {
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into(),
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        });
    }

    Ok(())
}

const C_PREAMBLE: &str = r#"
#ifdef __cplusplus
extern "C" {
#endif
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#ifdef __cplusplus
}
#endif
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if LUA_VERSION_NUM == 501
  #define LUA_OK 0
#endif

static lua_State *globalL = NULL;

static void lstop(lua_State *L, lua_Debug *ar) {
  (void)ar;
  lua_sethook(L, NULL, 0, 0);
  luaL_error(L, "interrupted!");
}

static void laction(int i) {
  signal(i, SIG_DFL);
  lua_sethook(globalL, lstop, LUA_MASKCALL | LUA_MASKRET | LUA_MASKCOUNT, 1);
}

static void createargtable(lua_State *L, char **argv, int argc, int script) {
  int i, narg;
  if (script == argc) script = 0;
  narg = argc - (script + 1);
  lua_createtable(L, narg, script + 1);
  for (i = 0; i < argc; i++) {
    lua_pushstring(L, argv[i]);
    lua_rawseti(L, -2, i - script);
  }
  lua_setglobal(L, "arg");
}

static int msghandler(lua_State *L) {
  const char *msg = lua_tostring(L, 1);
  if (msg == NULL) {
    if (luaL_callmeta(L, 1, "__tostring") && lua_type(L, -1) == LUA_TSTRING)
      return 1;
    msg = lua_pushfstring(L, "(error object is a %s value)", luaL_typename(L, 1));
  }
  lua_getglobal(L, "debug");
  lua_getfield(L, -1, "traceback");
  lua_remove(L, -2);
  lua_pushstring(L, msg);
  lua_remove(L, -3);
  lua_pushinteger(L, 2);
  lua_call(L, 2, 1);
  return 1;
}

static int docall(lua_State *L, int narg, int nres) {
  int status;
  int base = lua_gettop(L) - narg;
  lua_pushcfunction(L, msghandler);
  lua_insert(L, base);
  globalL = L;
  signal(SIGINT, laction);
  status = lua_pcall(L, narg, nres, base);
  signal(SIGINT, SIG_DFL);
  lua_remove(L, base);
  return status;
}

"#;

const LUA_LOADER_SOURCE: &str = r#"local args = {...}
local lua_bundle = args[1]

local function load_string(str, name)
	if _VERSION == "Lua 5.1" then
		return loadstring(str, name)
	else
		return load(str, name)
	end
end

local function lua_loader(name)
	local separator = package.config:sub(1, 1)
	name = name:gsub(separator, ".")
	local mod = lua_bundle[name] or lua_bundle[name .. ".init"]
	if mod then
		if type(mod) == "string" then
			local chunk, errstr = load_string(mod, name)
			if chunk then
				return chunk
			else
				error(
					("error loading module '%s' from static Lua bundle:\n\t%s"):format(name, errstr),
					0
				)
			end
		elseif type(mod) == "function" then
			return mod
		end
	else
		return ("\n\tno module '%s' in static Lua bundle"):format(name)
	end
end
table.insert(package.loaders or package.searchers, 2, lua_loader)

local unpack = unpack or table.unpack
"#;

#[cfg(test)]
mod tests {
    use super::*;

    use assert_fs::fixture::PathCopy;
    #[cfg(target_os = "linux")]
    use assert_fs::prelude::{PathChild, PathCreateDir};
    use assert_fs::TempDir;

    use crate::lua_installation::detect_installed_lua_version;
    use crate::{config::ConfigBuilder, lua_version::LuaVersion, tree::FlatDistTree};
    #[cfg(target_os = "linux")]
    use crate::{
        lockfile::{LocalPackage, LocalPackageHashes, LockConstraint},
        package::PackageSpec,
        remote_package_source::RemotePackageSource,
        rockspec::RockBinaries,
    };

    #[cfg(target_os = "linux")]
    fn mk_dummy_package(spec: PackageSpec) -> LocalPackage {
        let hashes = LocalPackageHashes {
            rockspec: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .unwrap(),
            source: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .unwrap(),
        };
        LocalPackage::from(
            &spec,
            LockConstraint::Unconstrained,
            RockBinaries::default(),
            RemotePackageSource::Test,
            None,
            hashes)
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_collect_installed_files() {
        let staging = TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let tree = FlatDistTree::new(staging.to_path_buf(), LuaVersion::Lua51, &config).unwrap();

        let pkg_a = mk_dummy_package(PackageSpec::new("foo".into(), "1.0.0-1".parse().unwrap()));
        let layout_a = tree.entrypoint(&pkg_a).unwrap();
        staging
            .child(layout_a.src.strip_prefix(staging.path()).unwrap())
            .create_dir_all()
            .unwrap();
        tokio::fs::write(layout_a.src.join("foo.lua"), "return {}")
            .await
            .unwrap();

        let pkg_b = mk_dummy_package(PackageSpec::new("bar".into(), "2.0.0-1".parse().unwrap()));
        let layout_b = tree.entrypoint(&pkg_b).unwrap();
        staging
            .child(layout_b.src.strip_prefix(staging.path()).unwrap())
            .create_dir_all()
            .unwrap();
        tokio::fs::write(layout_b.src.join("bar.lua"), "return {}")
            .await
            .unwrap();
        staging
            .child(layout_b.lib.strip_prefix(staging.path()).unwrap())
            .create_dir_all()
            .unwrap();
        tokio::fs::write(
            layout_b.lib.join(format!("bar.{}", c_dylib_extension())),
            "")
        .await
        .unwrap();

        {
            let lockfile = tree.lockfile().unwrap();
            let mut lockfile = lockfile.write_guard();
            lockfile.add_entrypoint(&pkg_a);
            lockfile.add_entrypoint(&pkg_b);
        }

        let tree = FlatDistTree::new(staging.to_path_buf(), LuaVersion::Lua51, &config).unwrap();
        let files = collect_installed_files(&tree).unwrap();

        assert_eq!(files.src.len(), 2);
        assert!(files
            .src
            .iter()
            .all(|(_, p)| p.extension().is_some_and(|e| e == "lua")));
        assert_eq!(files.lib.len(), 1);
        assert!(files.lib[0]
            .extension()
            .is_some_and(|e| e == c_dylib_extension()));
    }

    #[tokio::test]
    async fn test_collect_installed_files_from_sample_project() {
        let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/only-src/");
        let project_dir = TempDir::new().unwrap();
        project_dir.copy_from(&sample, &["**"]).unwrap();

        let project = Project::from_exact(project_dir.path()).unwrap().unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .build()
            .unwrap();

        let staging = TempDir::new().unwrap();
        let tree = FlatDistTree::new(
            staging.to_path_buf(),
            config.lua_version().cloned().unwrap(),
            &config)
        .unwrap();

        InstallProject::new()
            .project(&project)
            .config(&config)
            .tree(&tree)
            .build()
            .await
            .unwrap();

        let files = collect_installed_files(&tree).unwrap();

        let module_keys: Vec<&str> = files.src.iter().map(|(m, _)| m.as_str()).collect();

        assert!(
            module_keys.contains(&"main"),
            "expected 'main' in {module_keys:?}"
        );
        assert!(
            module_keys.contains(&"foo"),
            "expected 'foo' in {module_keys:?}"
        );
    }

    #[tokio::test]
    async fn test_dist_bin_from_lua_source_compiles_and_runs() {
        test_dist_bin_compiles_and_runs("resources/test/sample-projects/only-src/", "1").await
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn test_dist_bin_from_c_source_compiles_and_runs() {
        test_dist_bin_compiles_and_runs("resources/test/sample-projects/c-src/", "OK").await
    }

    async fn test_dist_bin_compiles_and_runs(sample_project_path: &str, expected_output: &str) {
        let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(sample_project_path);
        let project_dir = TempDir::new().unwrap();
        project_dir.copy_from(&sample, &["**"]).unwrap();

        let project = Project::from_exact(project_dir.path()).unwrap().unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .build()
            .unwrap();

        let staging = TempDir::new().unwrap();
        let tree = FlatDistTree::new(
            staging.to_path_buf(),
            config.lua_version().cloned().unwrap(),
            &config)
        .unwrap();

        let out_dir = TempDir::new().unwrap();
        let binary = out_dir.path().join(if cfg!(target_env = "msvc") {
            "sample-project.exe"
        } else {
            "sample-project"
        });

        DistProjectBin::new()
            .project(&project)
            .config(&config)
            .tree(&tree)
            .output(binary.clone())
            .compile()
            .await
            .unwrap();

        assert!(binary.is_file(), "binary not produced");

        let out = tokio::process::Command::new(&binary)
            .output()
            .await
            .unwrap();

        assert!(out.status.success(), "binary exited non-zero:\n{:?}", out);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected_output);
    }
}
