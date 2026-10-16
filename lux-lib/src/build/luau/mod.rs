mod analyze;
mod require;
mod shims;

use std::path::Path;

use darklua_core::{
    process,
    rules::{
        ConvertLuauNumber, MakeAssignmentLocal, RemoveAttribute, RemoveCompoundAssignment,
        RemoveContinue, RemoveIfExpression, RemoveInterpolatedString, RemoveTypes, Rule,
    },
    Configuration, Options, Resources,
};
use miette::Diagnostic;
use thiserror::Error;

use crate::{fs, lua_rockspec::LuaModule};

use super::utils::make_writable;

/// Errors from transpiling Luau source to Lua.
#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum LuauTranspileError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error("failed to transpile Luau source to Lua:\n{0}")]
    #[diagnostic(help("run `lx check` to validate the Luau sources"))]
    Transpile(String),
    #[error("failed to access in-memory resources during transpilation: {0}")]
    Resource(String),
    #[error("cannot transpile Luau code that uses unsupported standard library: {0}")]
    #[diagnostic(help("rewrite the code to use Lua-compatible standard library functions"))]
    Unsupported(String),
    #[error("cannot transpile bare alias `@{0}`")]
    #[diagnostic(help(
        "Luau alias entrypoint requires are not transpiled to Lua; require a submodule, e.g. `@{0}/foo`."
    ))]
    BareAlias(String),
}

/// Transpiles a `.luau` source into the `.lua` module at `target_module`,
/// flattening its `require` paths
/// and prepending any Luau standard library shims it requires.
pub(crate) async fn transpile_luau_to_lua_module(
    source: &Path,
    target_module: &LuaModule,
    target_dir: &Path,
) -> Result<(), LuauTranspileError> {
    let luau_content = fs::tokio::read_to_string(source).await?;
    let lua_content = require::flatten_requires(&transpile_luau(&luau_content)?, target_module)?;
    let lua_content = shims::inject(&lua_content)?;
    let target = target_dir.join(target_module.to_lua_path());
    if let Some(parent) = target.parent() {
        fs::tokio::create_dir_all(parent).await?;
    }
    fs::tokio::write(&target, lua_content).await?;
    make_writable(&target).await?;
    Ok(())
}

fn transpile_luau(luau_content: &str) -> Result<String, LuauTranspileError> {
    let resources = Resources::from_memory();
    let file = Path::new("src").join("module.luau");
    resources
        .write(&file, luau_content)
        .map_err(|err| LuauTranspileError::Resource(format!("{err:?}")))?;
    let mut config = Configuration::empty();
    for rule in luau_to_lua_rules() {
        config.push_rule(rule);
    }
    process(&resources, Options::new("src").with_configuration(config))
        .map_err(|err| LuauTranspileError::Transpile(err.to_string()))?;
    resources
        .get(&file)
        .map_err(|err| LuauTranspileError::Resource(format!("{err:?}")))
}

fn luau_to_lua_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::<RemoveTypes>::default(),
        Box::<RemoveCompoundAssignment>::default(),
        Box::<RemoveInterpolatedString>::default(),
        Box::<RemoveContinue>::default(),
        Box::<RemoveIfExpression>::default(),
        Box::<MakeAssignmentLocal>::default(),
        Box::<ConvertLuauNumber>::default(),
        Box::<RemoveAttribute>::default(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transpile_luau() {
        let out = shims::inject(
            &transpile_luau(
                r#"type Point = {x: number}
local p: Point = { x = 1 }
p.x += 2
local s = `hi {p.x}`
return s"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!out.contains("number"));
        assert!(!out.contains("+="));
        assert!(!out.contains('`'));
        assert!(!out.contains("type Point"));

        let lua = mlua::Lua::new();
        let result: String = lua.load(&out).eval().unwrap();
        assert_eq!(result, "hi 3");
    }

    #[tokio::test]
    async fn test_transpile_luau_to_lua_module() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("greeting.luau");
        std::fs::write(
            &source,
            r#"return table.concat(string.split("a,b,c", ","), "|")"#,
        )
        .unwrap();
        let target = temp.path().join("out");
        let module: LuaModule = "greeting".parse().unwrap();
        transpile_luau_to_lua_module(&source, &module, &target)
            .await
            .unwrap();

        let lua = std::fs::read_to_string(target.join("greeting.lua")).unwrap();
        assert!(lua.contains("string.split"));

        let lua_state = mlua::Lua::new();
        let result: String = lua_state.load(&lua).eval().unwrap();
        assert_eq!(result, "a|b|c");
    }
}
