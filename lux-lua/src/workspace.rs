use lux_lib::workspace::Workspace;
use mlua::ExternalResult;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};

use crate::lua_impls::WorkspaceLua;

#[derive(Clone, mlua_extras::UserData)]
pub(crate) struct WorkspaceModule;

impl Typed for WorkspaceModule {
    fn ty() -> Type {
        Type::named("WorkspaceModule")
    }
}

impl TypedUserData for WorkspaceModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Load the current workspace, if in a workspace");
        methods.add_function("current", |_, ()| {
            Ok(Workspace::current().into_lua_err()?.map(WorkspaceLua))
        });

        methods.document("Load the workspace in the given directory, if present");
        methods.param("path", "The workspace root");
        methods.add_function("new", |_, path: String| {
            Ok(Workspace::from_exact(path)
                .into_lua_err()?
                .map(WorkspaceLua))
        });
        methods.document(
            "Search for a workspace upwards from the given directory and load it, if present",
        );
        methods.param("path", "The directory to search upwards from");
        methods.add_function("new_fuzzy", |_, path: String| {
            Ok(Workspace::from(path).into_lua_err()?.map(WorkspaceLua))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for interacting with a Lux workspace");
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::WorkspaceModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "WorkspaceModule",
            build: || Type::class(TypedClassBuilder::new::<WorkspaceModule>()),
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::{assert::PathAssert, prelude::PathChild, TempDir};
    use mlua::Lua;

    fn create_fake_single_project_workspace() -> (TempDir, Lua) {
        let workspace = TempDir::new().unwrap();
        std::fs::write(
            workspace.join("lux.toml"),
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
            .set("workspace_location", workspace.path())
            .unwrap();

        (workspace, lua)
    }

    #[test]
    fn lua_api_test_current_workspace() {
        let (workspace, lua) = create_fake_single_project_workspace();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workspace).unwrap();

        lua.load(
            r#"
            local workspace = lux.workspace.current()
            assert(workspace, "workspace should not be nil")
            "#,
        )
        .exec()
        .unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
    }

    #[test]
    fn lua_api_test_workspace() {
        let (_workspace, lua) = create_fake_single_project_workspace();
        lua.load(
            r#"
            local config = lux.config.default()
            local config = config.builder()
                :lua_version("5.1")
                :build()

            local workspace = lux.workspace.new(workspace_location)
            assert(workspace, "workspace should not be nil")

            assert(workspace:lockfile_path() == workspace_location .. "/lux.lock", "workspace.lockfile_path should be correct")
            assert(workspace:root() == workspace_location, "workspace.root should be correct")
            assert(workspace:tree(config), "workspace.tree should not be nil")
            assert(workspace:test_tree(config), "workspace.test_tree should not be nil")
            assert(workspace:members(), "workspace.members should not be nil")
            assert(workspace:single_member_or_select("test-package"), "workspace.single_member_or_select('test-package') should not be nil")

            workspace = lux.workspace.new(workspace_location .. "/nonexistent")
            assert(not workspace, "workspace should be nil")
            "#,
        )
        .exec()
        .unwrap();
    }
}
