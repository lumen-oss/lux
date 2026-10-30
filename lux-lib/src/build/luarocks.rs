use crate::build::backend::BuildInfo;
use crate::build::backend::RunBuildArgs;
use crate::fs;
use crate::rockspec;
use crate::rockspec::LuaVersionCompatibility;
use crate::rockspec::Rockspec;
use crate::tree;
use crate::tree::InstallTree;
use std::path::Path;

use crate::{
    config::Config,
    lockfile::LockedPackageSpec,
    luarocks::{self, install_rock_manifest, installation::exec, RockManifestSources},
};

use crate::fs::tempfile::tempdir;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("error instantiating luarocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] luarocks::installation::Error),
    #[error("error running 'luarocks make'")]
    #[diagnostic(forward(0))]
    Exec(#[from] exec::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InstallRockManifest(#[from] luarocks::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Rockspec(Box<dyn Diagnostic + Send + Sync>),
    #[error("error installing luarocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaVersion(#[from] rockspec::Error),
}

#[tracing::instrument(name = "Delegating to luarocks",
    skip_all,
    level = "info"
    fields(backend = build_backend_name),
)]
pub(crate) async fn build<R: Rockspec, T: InstallTree>(
    build_backend_name: &str,
    rockspec: &R,
    args: RunBuildArgs<'_, T>,
) -> Result<BuildInfo, Error> {
    let package = args.package;
    let lua = args.lua;
    let config = args.config;
    let build_dir = args.build_dir;
    let tree = args.tree;

    let rockspec_temp_dir = tempdir()?;
    let rockspec_file = rockspec_temp_dir.path().join(format!(
        "{}-{}.rockspec",
        rockspec.package(),
        rockspec.version()
    ));
    fs::tokio::write(
        &rockspec_file,
        rockspec
            .to_lua_remote_rockspec_string()
            .map_err(|err| Error::Rockspec(Box::new(err)))?,
    )
    .await?;
    let luarocks = luarocks::Installation::new(config, tree.build_tree(config)?)?;
    let luarocks_tree = tempdir()?;
    luarocks
        .make(&rockspec_file, build_dir, luarocks_tree.path(), lua)
        .await?;
    install(rockspec, luarocks_tree.path(), tree, package, config).await
}

async fn install<R: Rockspec, T: InstallTree>(
    rockspec: &R,
    luarocks_tree: &Path,
    tree: &T,
    package: &LockedPackageSpec,
    config: &Config,
) -> Result<BuildInfo, Error> {
    let layout = tree.layout_for(package);
    let lua_version = rockspec.lua_version_matches(config)?;
    fs::tokio::create_dir_all(tree.bin()).await?;
    let lua_version = lua_version.version_compatibility_str();
    let package_dir = luarocks_tree
        .join("lib")
        .join("luarocks")
        .join(format!("rocks-{lua_version}"))
        .join(format!("{}", rockspec.package()))
        .join(format!("{}", rockspec.version()));
    let sources = RockManifestSources {
        root: package_dir,
        lib: luarocks_tree.join("lib").join("lua").join(&lua_version),
        lua: luarocks_tree.join("share").join("lua").join(&lua_version),
        bin: luarocks_tree.join("bin"),
    };
    install_rock_manifest(sources, &layout, &tree.bin()).await?;
    Ok(BuildInfo::default())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::rockspec;

    use assert_fs::TempDir;

    use crate::{
        config,
        lockfile::{LockConstraint, LockedPackageSpec, OptState, PinnedState},
        lua,
        rockspec::Rockspec,
        tree::InstallTree,
    };

    use super::install;

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[tokio::test]
    async fn installs_all_build_artifacts() {
        let rockspec = rockspec::lua::Remote::new(
            r#"
rockspec_format = "3.0"
package = "test-luarocks-backend"
version = "1.0-1"
source = { url = "https://github.com/lumen-oss/luarocks-stub" }
build = { type = "test-custom" }
"#,
        )
        .unwrap();
        let tree_dir = TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .lua_version(Some(lua::Version::Lua51))
            .user_tree(Some(tree_dir.path().to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let package = LockedPackageSpec::new(
            rockspec.package(),
            rockspec.version(),
            LockConstraint::Unconstrained,
            Vec::new(),
            Vec::new(),
            &PinnedState::default(),
            &OptState::default(),
        );
        tree.prepare(&package).unwrap();

        let luarocks_tree = TempDir::new().unwrap();
        let package_dir = luarocks_tree
            .path()
            .join("lib/luarocks/rocks-5.1/test-luarocks-backend/1.0-1");
        write_file(
            &package_dir,
            "rock_manifest",
            r#"
rock_manifest = {
    lib = { ["test_luarocks_backend.so"] = "00000000000000000000000000000000" },
    lua = { ["test_luarocks_backend/init.lua"] = "00000000000000000000000000000000" },
    bin = { tool = "00000000000000000000000000000000" },
    doc = { ["README.md"] = "00000000000000000000000000000000" },
    conf = { ["config.toml"] = "00000000000000000000000000000000" },
    plugin = { ["foo.lua"] = "00000000000000000000000000000000" },
    ["test-luarocks-backend-1.0-1.rockspec"] = "00000000000000000000000000000000",
}
"#,
        );
        write_file(
            luarocks_tree.path(),
            "lib/lua/5.1/test_luarocks_backend.so",
            "",
        );
        write_file(
            luarocks_tree.path(),
            "share/lua/5.1/test_luarocks_backend/init.lua",
            "return true",
        );
        write_file(
            luarocks_tree.path(),
            "share/lua/5.1/unlisted.lua",
            "return true",
        );
        write_file(luarocks_tree.path(), "bin/tool", "#!/bin/sh\n");
        write_file(&package_dir, "doc/README.md", "docs");
        write_file(&package_dir, "conf/config.toml", "conf");
        write_file(&package_dir, "plugin/foo.lua", "return true");
        write_file(
            &package_dir,
            "test-luarocks-backend-1.0-1.rockspec",
            "rockspec",
        );
        write_file(&package_dir, "unlisted.txt", "not in the manifest");

        install(&rockspec, luarocks_tree.path(), &tree, &package, &config)
            .await
            .unwrap();

        let layout = tree.layout_for(&package);
        assert!(layout.lib.join("test_luarocks_backend.so").is_file());
        assert!(layout
            .src
            .join("test_luarocks_backend")
            .join("init.lua")
            .is_file());
        assert!(tree.bin().join("tool").is_file());
        assert!(layout.doc.join("README.md").is_file());
        assert!(layout.conf.join("config.toml").is_file());
        assert!(layout.etc.join("plugin").join("foo.lua").is_file());
        assert!(layout
            .etc
            .join("test-luarocks-backend-1.0-1.rockspec")
            .is_file());
        assert!(!layout.etc.join("rock_manifest").exists());
        assert!(!layout.etc.join("unlisted.txt").exists());
        assert!(!layout.src.join("unlisted.lua").exists());
    }
}
