use std::path::PathBuf;

use lux_lib::project::Project;
use mlua::{ExternalResult, Lua, Table};

use crate::lua_impls::ProjectLua;

pub fn project(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    table.set(
        "current",
        lua.create_function(|_, ()| Ok(Project::current().into_lua_err()?.map(ProjectLua)))?,
    )?;

    table.set(
        "new",
        lua.create_function(|_, path: PathBuf| {
            Ok(Project::from_exact(path).into_lua_err()?.map(ProjectLua))
        })?,
    )?;

    table.set(
        "new_fuzzy",
        lua.create_function(|_, path: PathBuf| {
            Ok(Project::from(path).into_lua_err()?.map(ProjectLua))
        })?,
    )?;

    Ok(table)
}

#[cfg(test)]
mod tests {
    use assert_fs::{assert::PathAssert, prelude::PathChild, TempDir};
    use mlua::Lua;

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
    fn lua_api_test_current_project() {
        let (project, lua) = create_fake_project();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project).unwrap();

        lua.load(
            r#"
            local project = lux.project.current()
            assert(project, "project should not be nil")
            "#,
        )
        .exec()
        .unwrap();

        std::env::set_current_dir(old_cwd).unwrap();
    }

    #[test]
    fn lua_api_test_project() {
        let (project, lua) = create_fake_project();

        lua.load(
            r#"
            local config = lux.config.default()
            local config = config.builder()
                :lua_version("5.1")
                :build()

            local project = lux.project.new(project_location)
            assert(project, "project should not be nil")

            assert(project:toml_path() == project_location .. "/lux.toml", "project.toml_path should be correct")
            assert(project:extra_rockspec_path() == project_location .. "/extra.rockspec", "project.extra_rockspec_path should be correct")
            assert(project:lockfile_path() == project_location .. "/lux.lock", "project.lockfile_path should be correct")
            assert(project:root() == project_location, "project.root should be correct")
            assert(project:lua_version(config) == "5.1", "project.lua_version should be correct")
            assert(project:toml(), "project.toml should not be nil")
            assert(project:local_rockspec(), "project.local_rockspec should not be nil")
            assert(project:remote_rockspec(), "project.remote_rockspec should not be nil")
            assert(not project:extra_rockspec(), "project.extra_rockspec should be nil")
            assert(project:tree(config), "project.tree should not be nil")
            assert(project:test_tree(config), "project.test_tree should not be nil")

            project = lux.project.new(project_location .. "/nonexistent")
            assert(not project, "project should be nil")
            "#,
        )
        .exec()
        .unwrap();

        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }

        // ADDING DEPENDENCIES

        lua.load(
            r#"
            local co = coroutine.create(function()
                local config = lux.config.default()

                local project = lux.project.new(project_location)
                assert(project, "project should not be nil")

                local deps = { "say == 1.3" }

                project:add({ regular = deps, }, config)
                project:add({ build = deps, }, config)
                project:add({ test = deps, }, config)
            end)

            -- Halt main thread until coroutine is done
            while coroutine.status(co) ~= "dead" do
                coroutine.resume(co)
            end
        "#,
        )
        .exec()
        .unwrap();

        let toml_path = project.child("lux.toml");
        toml_path.assert(
            r#"
package = "test-package"
version = "0.1.0"
lua = "5.1"

[dependencies]
say = "==1.3"

[build_dependencies]
say = "==1.3"

[test_dependencies]
say = "==1.3"

[source]
url = "https://example.com/test/test"

[build]
type = "builtin"
"#,
        );

        // REMOVING DEPENDENCIES

        lua.load(
            r#"
            local co = coroutine.create(function()
                local config = lux.config.default()

                local project = lux.project.new(project_location)
                assert(project, "project should not be nil")

                local deps = { "say" }

                project:remove({ regular = deps }, config)
                project:remove({ build = deps }, config)
                project:remove({ test = deps }, config)
            end)

            -- Halt main thread until coroutine is done
            while coroutine.status(co) ~= "dead" do
                coroutine.resume(co)
            end
        "#,
        )
        .exec()
        .unwrap();

        toml_path.assert(
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
        );
    }
}
