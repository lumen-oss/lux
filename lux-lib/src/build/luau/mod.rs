use std::path::Path;

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
    Transpile(#[from] kalua::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
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
    let lua_content = kalua::transpile(&luau_content, target_module.as_str())?;
    let target = target_dir.join(target_module.to_lua_path());
    if let Some(parent) = target.parent() {
        fs::tokio::create_dir_all(parent).await?;
    }
    fs::tokio::write(&target, lua_content).await?;
    make_writable(&target).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
