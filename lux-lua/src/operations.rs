//! Functions for interacting with global state (currently installed packages user-wide,
//! getting all packages from the manifest, etc.)

use lux_lib::package;
use std::{collections::HashMap, path::PathBuf};

use itertools::Itertools;
use lux_lib::tree::InstallTree;
use lux_lib::{
    drivers::sync::{Sync, SyncMode, TargetSet},
    drivers::InstallPackages,
    lockfile::LockedPackageId,
    lua,
    operations::{self, pin},
    package::db::PackageDB,
    rockspec::dependency,
    tree::{RockMatches, Tree},
};
use mlua::prelude::*;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};
use path_slash::PathBufExt;

use crate::lua_impls::{
    self, AddDependenciesLua, ConfigLua, DependencyKindLua, DownloadedRockspecLua,
    LockedPackageIdLua, LockedPackageLua, PackageInstallSpecLua, PackageNameLua, PinnedStateLua,
    ProjectLua, SyncReportLua, TreeLua, WorkspaceLua,
};

#[derive(Clone)]
pub(crate) struct OperationsModule;

impl Typed for OperationsModule {
    fn ty() -> Type {
        Type::named("operations.Module")
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
                let _runtime = lua::runtime().enter();
                search(query, config).await
            },
        );

        methods.document("List installed packages that have newer versions available");
        methods.param("workspace", "Workspace to inspect");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "outdated",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                let tree = workspace.0.tree(&config.0).into_lua_err()?;
                let package_db = PackageDB::from_config(&config.0).await.into_lua_err()?;
                let rocks = tree.as_rock_list().into_lua_err()?;
                let mut outdated: HashMap<String, Vec<HashMap<String, String>>> = HashMap::new();
                for rock in rocks.iter() {
                    if let Ok(Some(latest)) = rock.to_package().has_update(&package_db).await {
                        outdated
                            .entry(rock.name().to_string())
                            .or_default()
                            .push(HashMap::from([
                                ("current".to_string(), rock.version().to_string()),
                                ("latest".to_string(), latest.to_string()),
                            ]));
                    }
                }
                Ok(outdated)
            },
        );

        methods.document("Install one or multiple package(s)");
        methods.param("packages", "List of packages to install");
        methods.param("tree", "Install tree");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "install",
            |_, (packages, tree, config): (Vec<PackageInstallSpecLua>, TreeLua, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                let specs = packages.into_iter().map(|p| p.0).collect();
                let package_db = PackageDB::from_config(&config.0).await.into_lua_err()?;
                InstallPackages::new(&config.0, &tree.0)
                    .package_db(package_db)
                    .packages(specs)
                    .install()
                    .await
                    .into_lua_err()
                    .map(|(pkgs, _)| pkgs.into_iter().map(LockedPackageLua).collect::<Vec<_>>())
            },
        );

        methods.document("operations::Uninstall one or multiple package(s)");
        methods.param("packages", "IDs of packages to uninstall");
        methods.param("tree", "Install tree");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "uninstall",
            |_, (packages, tree, config): (Vec<LockedPackageIdLua>, Option<TreeLua>, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                let ids = packages.into_iter().map(|p| p.0);
                operations::Uninstall::new()
                    .config(&config.0)
                    .packages(ids)
                    .maybe_tree(tree.map(|tree| tree.0))
                    .remove()
                    .await
                    .into_lua_err()
                    .map(|ids| ids.into_iter().map(LockedPackageIdLua).collect::<Vec<_>>())
            },
        );

        methods.document("operations::Update installed packages in a workspace");
        methods.param("workspace", "Workspace to update packages in");
        methods.param(
            "packages",
            "Optional list of packages to update (e.g. {'foo', 'bar >= 1.0'})",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "update",
            |_, (workspace, packages, config): (WorkspaceLua, Option<Vec<String>>, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                let package_reqs = packages
                    .map(|packages| {
                        packages
                            .into_iter()
                            .map(|s| s.parse())
                            .collect::<Result<Vec<package::Req>, _>>()
                    })
                    .transpose()
                    .into_lua_err()?;

                operations::Update::new(&config.0)
                    .workspace(workspace.0)
                    .packages(package_reqs)
                    .update()
                    .await
                    .into_lua_err()
                    .map(|pkgs| pkgs.into_iter().map(LockedPackageLua).collect::<Vec<_>>())
            },
        );

        // TODO(vhyrro): Add the ability to specify a project
        methods.document("Add dependencies to a workspace project and install them immediately");
        methods.param("workspace", "Workspace containing the project to modify");
        methods.param(
            "deps",
            "Dependencies to add, e.g. { regular = {'foo', 'bar >= 1.0'} }",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "add",
            |_,
             (mut workspace, deps, config): (
                WorkspaceLua,
                AddDependenciesLua,
                ConfigLua,
            )| async move {
                let _runtime = lua::runtime().enter();
                let package_db = PackageDB::from_config(&config.0).await.into_lua_err()?;

                {
                    let project = workspace
                        .0
                        .single_member_or_select_mut(&None)
                        .into_lua_err()?;
                    lua_impls::project_add_dependencies(project, deps.0, &package_db, &config.0)
                        .await?;
                }

                let report = Sync::new(&workspace.0, &config.0)
                    .mode(SyncMode::Open)
                    .targets(TargetSet::all(&workspace.0, true))
                    .only_deps(true)
                    .sync()
                    .await
                    .into_lua_err()?;

                Ok(report
                    .added()
                    .iter()
                    .cloned()
                    .map(LockedPackageLua)
                    .collect::<Vec<_>>())
            },
        );

        methods.document(
            "Remove dependencies from a workspace project and uninstall them immediately",
        );
        methods.param("workspace", "Workspace containing the project to modify");
        methods.param(
            "deps",
            "Dependencies to remove, e.g. { regular = {'foo', 'bar'} }",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "remove",
            |_,
             (mut workspace, deps, config): (
                WorkspaceLua,
                DependencyKindLua<PackageNameLua>,
                ConfigLua,
            )| async move {
                let _runtime = lua::runtime().enter();
                let deps = lua_impls::map_dependency_type_names(deps.0);
                let tree = workspace.0.tree(&config.0).into_lua_err()?;
                let ids_to_remove: Vec<LockedPackageId> = deps_to_ids(&deps, &tree);

                if ids_to_remove.is_empty() {
                    return Ok(Vec::new());
                }
                workspace
                    .0
                    .single_member_or_select_mut(&None)
                    .into_lua_err()?
                    .remove(deps.as_ref())
                    .await
                    .into_lua_err()?;

                operations::Uninstall::new()
                    .config(&config.0)
                    .tree(tree)
                    .packages(ids_to_remove)
                    .remove()
                    .await
                    .into_lua_err()
                    .map(|ids| ids.into_iter().map(LockedPackageIdLua).collect::<Vec<_>>())
            },
        );

        methods.document("Sync all workspace dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .mode(SyncMode::Frozen)
                    .targets(TargetSet::all(&workspace.0, true))
                    .sync()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)
            },
        );

        methods.document("Sync workspace dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "sync_dependencies",
            |_, (workspace, config): (WorkspaceLua, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                Sync::new(&workspace.0, &config.0)
                    .mode(SyncMode::Frozen)
                    .targets(TargetSet::all(&workspace.0, false))
                    .sync()
                    .await
                    .into_lua_err()
                    .map(SyncReportLua)
            },
        );

        methods.document("Sync workspace test dependencies");
        methods.param("workspace", "Workspace to sync");
        methods.param("config", "Lux config");

        methods.document("Build a workspace");
        methods.param("workspace", "Workspace to build");
        methods.param("package", "Build only this package");
        methods.param("config", "Lux config");
        methods.add_async_function(
            "build",
            |_, (workspace, package, config): (WorkspaceLua, Option<PackageNameLua>, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                operations::BuildWorkspace::new(&workspace.0, &config.0)
                    .maybe_package(package.map(|p| p.0))
                    .no_lock(false)
                    .only_deps(false)
                    .build()
                    .await
                    .into_lua_err()
                    .map(|packages: Vec<_>| packages.into_iter().map(LockedPackageLua).collect_vec())
            },
        );

        methods.document("Distribute a project as a single static binary");
        methods.param("project", "Project to compile");
        methods.param("tree", "Install tree");
        methods.param("output", "Destination path for the compiled binary");
        methods.param("config", "Lux config");
        methods.ret("the path to the compiled binary");
        methods.add_async_function(
            "dist_bin",
            |_, (project, tree, output, config): (ProjectLua, TreeLua, String, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                operations::dist::ProjectBin::new()
                    .project(&project.0)
                    .tree(&tree.0)
                    .output(PathBuf::from(output))
                    .config(&config.0)
                    .compile()
                    .await
                    .into_lua_err()
                    .map(|path| path.to_slash_lossy().to_string())
            },
        );

        methods.document("operations::Download the RockSpec for a package");
        methods.param(
            "package_req",
            "Package to search for, e.g. 'foo' or 'foo >= 1.0.0'",
        );
        methods.param("config", "Lux config");
        methods.add_async_function(
            "download_rockspec",
            |_, (package_req, config): (String, ConfigLua)| async move {
                let _runtime = lua::runtime().enter();
                let req = package_req.parse().into_lua_err()?;
                operations::Download::new(&req, &config.0)
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
            |_, (package_id, tree, pin_state): (LockedPackageIdLua, TreeLua, PinnedStateLua)| {
                pin::set_state(&package_id.0, &tree.0, pin_state.0).into_lua_err()
            },
        );
    }

    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("build::backend::Module for Lux operations");
    }
}

impl mlua::UserData for OperationsModule {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::OperationsModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "operations.Module",
            build: || Type::class(TypedClassBuilder::new::<OperationsModule>().build()),
        }
    }
}

async fn search(query: String, config: ConfigLua) -> mlua::Result<HashMap<String, Vec<String>>> {
    let remote_db = PackageDB::from_config(&config.0).await.into_lua_err()?;

    Ok(remote_db
        .search(&query.parse().into_lua_err()?)
        .await
        .into_lua_err()?
        .into_iter()
        .map(|(name, versions)| {
            (
                name.to_string(),
                versions.into_iter().map(|v| v.to_string()).collect(),
            )
        })
        .collect())
}

fn deps_to_ids(deps: &dependency::Kind<package::Name>, tree: &Tree) -> Vec<LockedPackageId> {
    let names = match deps {
        dependency::Kind::Regular(d) | dependency::Kind::Build(d) | dependency::Kind::Test(d) => d,
        dependency::Kind::External(_) => &Vec::default(),
    };

    names
        .iter()
        .filter_map(|name| {
            let req = package::Req::parse(name.to_string().as_str()).ok()?;
            tree.match_rocks(&req).ok()
        })
        .flat_map(|matches| match matches {
            RockMatches::Single(id) => vec![id],
            RockMatches::Many(ids) => ids.into(),
            _ => vec![],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use assert_fs::TempDir;
    #[cfg(feature = "impure_tests")]
    use flaky_test::flaky_test;
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
            assert(type(ops.build)             == "function")
            assert(type(ops.download_rockspec) == "function")
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
            .load(r#"{ package = "busted >= 2.0", entry_type = "dependency_only", pin = true, opt = true, build_behaviour = "ignore" }"#)
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

    #[cfg(feature = "impure_tests")]
    #[flaky_test(times = 5)]
    fn test_sync_report_shape() {
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
                local workspace = lux.workspace.new(project_location)
                report = lux.operations.sync(workspace, config)
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

    #[cfg(feature = "impure_tests")]
    #[flaky_test(times = 5)]
    fn test_downloaded_rockspec_shape() {
        let lua = setup_lua();
        lua.load(
            r#"
            local downloaded
            local co = coroutine.create(function()
                local config = lux.config.default()
                downloaded = lux.operations.download_rockspec("say >= 1.3", config)
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
