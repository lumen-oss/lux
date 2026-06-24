//! Functions for interacting with global state (currently installed packages user-wide,
//! getting all packages from the manifest, etc.)

use std::collections::HashMap;

use itertools::Itertools;
use lux_lib::{
    lua::lua_runtime,
    operations::{set_pinned_state, BuildWorkspace, Download, Install, Sync, Uninstall, Update},
    progress::{Progress, ProgressBar},
    remote_package_db::RemotePackageDB,
};
use mlua::prelude::*;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};

use crate::lua_impls::{
    ConfigLua, DownloadedRockspecLua, LocalPackageIdLua, LocalPackageLua, PackageInstallSpecLua,
    PackageNameLua, PinnedStateLua, SyncReportLua, TreeLua, WorkspaceLua,
};

#[derive(Clone, mlua_extras::UserData)]
pub(crate) struct OperationsModule;

impl Typed for OperationsModule {
    fn ty() -> Type {
        Type::named("OperationsModule")
    }
}

impl TypedUserData for OperationsModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Search for a remote package");
        methods.param(
            "query",
            "Package to search for, e.g. 'foo' or 'foo >= 1.0.0'",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "search",
            |_, (query, config): (String, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                search(query, config).await
            },
        );

        methods.document("Install one or multiple package(s)");
        methods.param("packages", "List of packages to install");
        methods.param("tree", "Install tree");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "install",
            |_, (packages, tree, config): (Vec<PackageInstallSpecLua>, TreeLua, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                let specs = packages.into_iter().map(|p| p.0).collect();
                Install::new(&config.0)
                    .packages(specs)
                    .tree(tree.0)
                    .install()
                    .await
                    .into_lua_err()
                    .map(|pkgs| pkgs.into_iter().map(LocalPackageLua).collect::<Vec<_>>())
            },
        );

        methods.document("Uninstall one or multiple package(s)");
        methods.param("packages", "IDs of packages to uninstall");
        methods.param("tree", "Install tree");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "uninstall",
            |_, (packages, tree, config): (Vec<LocalPackageIdLua>, Option<TreeLua>, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                let ids = packages.into_iter().map(|p| p.0);
                Uninstall::new()
                    .config(&config.0)
                    .packages(ids)
                    .maybe_tree(tree.map(|tree| tree.0))
                    .remove()
                    .await
                    .into_lua_err()
            },
        );

        methods.document("Update installed packages");
        methods.param("config", "Lux config");
        methods.add_async_function("update", |_, config: ConfigLua| async move {
            let _runtime = lua_runtime().enter();
            Update::new(&config.0)
                .update()
                .await
                .into_lua_err()
                .map(|pkgs| pkgs.into_iter().map(LocalPackageLua).collect::<Vec<_>>())
        });

        methods.document("Sync all workspace dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .sync_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)?;
                Sync::new(&workspace.0, &config.0)
                    .sync_build_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)?;
                Sync::new(&workspace.0, &config.0)
                    .sync_test_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)?;
                Ok(())
            },
        );

        methods.document("Sync workspace dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync_dependencies",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .sync_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)
            },
        );

        methods.document("Sync workspace build dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync_build_dependencies",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .sync_build_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)
            },
        );

        methods.document("Sync workspace test dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync_test_dependencies",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .sync_test_dependencies()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)
            },
        );

        methods.document("Build a workspace");
        methods.param("workspace", "Workspace to build");
        methods.param("package", "Build only this package");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "build",
            |_, (workspace, package, config): (WorkspaceLua, Option<PackageNameLua>, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                BuildWorkspace::new(&workspace.0, &config.0)
                    .maybe_package(package.map(|p| p.0))
                    .no_lock(false)
                    .only_deps(false)
                    .build()
                    .await
                    .into_lua_err()
                    .map(|packages: Vec<_>| packages.into_iter().map(LocalPackageLua).collect_vec())
            },
        );

        methods.document("Download the RockSpec for a package");
        methods.param(
            "package_req",
            "Package to search for, e.g. 'foo' or 'foo >= 1.0.0'",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "download_rockspec",
            |_, (package_req, config): (String, ConfigLua)| async move {
                let _runtime = lua_runtime().enter();
                let req = package_req.parse().into_lua_err()?;
                let progress = Progress::<ProgressBar>::no_progress();
                Download::new(&req, &config.0, &progress)
                    .download_rockspec()
                    .await
                    .into_lua_err()
                    .map(DownloadedRockspecLua)
            },
        );

        methods.document("Set the pinned state of a package");
        methods.param("package_id", "ID of the package to pin");
        methods.param("tree", "Install tree");
        methods.param("pin_state", "The pinned state to set");
        methods.add_function(
            "pin",
            |_, (package_id, tree, pin_state): (LocalPackageIdLua, TreeLua, PinnedStateLua)| {
                set_pinned_state(&package_id.0, &tree.0, pin_state.0).into_lua_err()
            },
        );
    }

    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for Lux operations");
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::OperationsModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "OperationsModule",
            build: || Type::class(TypedClassBuilder::new::<OperationsModule>()),
        }
    }
}

async fn search(query: String, config: ConfigLua) -> mlua::Result<HashMap<String, Vec<String>>> {
    let remote_db =
        RemotePackageDB::from_config(&config.0, &Progress::<ProgressBar>::no_progress())
            .await
            .into_lua_err()?;

    Ok(remote_db
        .search(&query.parse().into_lua_err()?)
        .into_iter()
        .map(|(name, versions)| {
            (
                name.to_string(),
                versions.into_iter().map(|v| v.to_string()).collect(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use assert_fs::TempDir;
    use mlua::{FromLua, Lua, LuaSerdeExt};

    use crate::lua_impls::PackageInstallSpecLua;

    fn setup_lua() -> Lua {
        let lua = Lua::new();
        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        lua
    }

    fn create_fake_project() -> (TempDir, Lua) {
        let project = TempDir::new().unwrap();
        std::fs::write(
            project.join("lux.toml"),
            r#"
package = "test-package"
version = "0.1.0"
lua = "5.1"

[dependencies]

[build_dependencies]

[test_dependencies]

[source]
url = "https://example.com/test/test"

[build]
type = "builtin"
"#,
        )
        .unwrap();

        let lua = Lua::new();
        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        lua.globals()
            .set("project_location", project.path())
            .unwrap();

        (project, lua)
    }

    #[test]
    fn test_operations_table_shape() {
        let lua = setup_lua();
        lua.load(
            r#"
            local ops = lux.operations
            assert(type(ops.install)   == "function")
            assert(type(ops.uninstall) == "function")
            assert(type(ops.update)    == "function")
            assert(type(ops.sync)      == "function")
            assert(type(ops.build)     == "function")
            assert(type(ops.download)  == "function")
            assert(type(ops.pin)       == "function")
            assert(type(ops.search)    == "function")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn test_package_install_spec_from_string() {
        let lua = Lua::new();
        let value = lua.to_value("say >= 1.3").unwrap();
        PackageInstallSpecLua::from_lua(value, &lua).unwrap();
    }

    #[test]
    fn test_package_install_spec_from_full_table() {
        let lua = Lua::new();
        let value = lua
            .load(r#"{ package = "busted >= 2.0", entry_type = "dependency_only", pin = true, opt = true, build_behaviour = "no_force" }"#)
            .eval()
            .unwrap();
        PackageInstallSpecLua::from_lua(value, &lua).unwrap();
    }

    #[test]
    fn test_package_install_spec_table_defaults() {
        let lua = Lua::new();
        let value = lua.load(r#"{ package = "inspect" }"#).eval().unwrap();
        PackageInstallSpecLua::from_lua(value, &lua).unwrap();
    }

    #[test]
    fn test_package_install_spec_invalid_entry_type() {
        let lua = Lua::new();
        let value = lua
            .load(r#"{ package = "say", entry_type = "invalid" }"#)
            .eval()
            .unwrap();
        assert!(PackageInstallSpecLua::from_lua(value, &lua).is_err());
    }

    #[test]
    fn test_package_install_spec_invalid_package_req() {
        let lua = Lua::new();
        let value = lua.to_value("!!invalid").unwrap();
        assert!(PackageInstallSpecLua::from_lua(value, &lua).is_err());
    }

    #[test]
    fn test_package_install_spec_invalid_build_behaviour() {
        let lua = Lua::new();
        let value = lua
            .load(r#"{ package = "say", build_behaviour = "invalid" }"#)
            .eval()
            .unwrap();
        assert!(PackageInstallSpecLua::from_lua(value, &lua).is_err());
    }

    #[test]
    fn test_sync_report_shape() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            return;
        }

        let tree = TempDir::new().unwrap();
        let (_project, lua) = create_fake_project();
        lua.globals().set("tree", tree.path()).unwrap();

        lua.load(
            r#"
            local report
            local co = coroutine.create(function()
                local config = lux.config.builder()
                    :lua_version("5.1")
                    :user_tree(tree)
                    :build()
                local project = lux.project.new(project_location)
                report = lux.operations.sync(project, config)
            end)

            while coroutine.status(co) ~= "dead" do
                coroutine.resume(co)
            end

            assert(report)
            assert(type(report.added)   == "table")
            assert(type(report.removed) == "table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn test_downloaded_rockspec_shape() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            return;
        }

        let lua = setup_lua();
        lua.load(
            r#"
            local downloaded
            local co = coroutine.create(function()
                local config = lux.config.default()
                downloaded = lux.operations.download("say >= 1.3", config)
            end)

            while coroutine.status(co) ~= "dead" do
                coroutine.resume(co)
            end

            assert(downloaded)
            assert(downloaded:rockspec())
            assert(downloaded:rockspec():package() == "say")
        "#,
        )
        .exec()
        .unwrap();
    }
}
