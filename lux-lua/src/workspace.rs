use std::path::PathBuf;

use lux_lib::workspace::Workspace;
use mlua::{ExternalResult, Lua, Table};

use crate::lua_impls::WorkspaceLua;

pub fn workspace(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    table.set(
        "current",
        lua.create_function(|_, ()| Ok(Workspace::current().into_lua_err()?.map(WorkspaceLua)))?,
    )?;

    table.set(
        "new",
        lua.create_function(|_, path: PathBuf| {
            Ok(Workspace::from_exact(path)
                .into_lua_err()?
                .map(WorkspaceLua))
        })?,
    )?;

    table.set(
        "new_fuzzy",
        lua.create_function(|_, path: PathBuf| {
            Ok(Workspace::from(path).into_lua_err()?.map(WorkspaceLua))
        })?,
    )?;

    Ok(table)
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
            assert(workspace:try_members("test-package"), "workspace.try_members('test-package') should not be nil")
            assert(workspace:try_member("test-package"), "workspace.try_member('test-package') should not be nil")

            workspace = lux.workspace.new(workspace_location .. "/nonexistent")
            assert(not workspace, "workspace should be nil")
            "#,
        )
        .exec()
        .unwrap();
    }
}
