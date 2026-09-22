//! Transpile Luau source code to Lua.

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

/// Errors from transpiling Luau source to Lua.
#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to transpile Luau source to Lua:\n{0}")]
    #[diagnostic(help("validate the Luau sources before building"))]
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

/// - Transpiles Luau source into Lua for the given module.
/// - Flattens its `require` paths so that dependencies resolve as flat module names.
/// - Prepends Luau standard library shims required by the given Lua source.
///
/// Alias requires drop their `@scope/package` prefix:
///
/// - `require("@dep/foo")` -> `require("foo")`
/// - `require("@dep/foo/bar")` -> `require("foo.bar")`
///
/// Relative requires resolve against the module's own name:
///
/// - `require("./foo")` in module `main` -> `require("foo")`
/// - `require("./foo")` in module `a.b` -> `require("a.foo")`
/// - `require("../foo")` in module `a.b` -> `require("foo")`
///
/// A bare alias (`require("@dep")`) is a Luau alias entrypoint require and is rejected.
pub fn transpile(luau_content: &str, module: &str) -> Result<String, Error> {
    let lua_content = require::flatten_requires(&transpile_luau(luau_content)?, module)?;
    shims::inject(&lua_content)
}

fn transpile_luau(luau_content: &str) -> Result<String, Error> {
    let resources = Resources::from_memory();
    let file = Path::new("src").join("module.luau");
    resources
        .write(&file, luau_content)
        .map_err(|err| Error::Resource(format!("{err:?}")))?;
    let mut config = Configuration::empty();
    for rule in luau_to_lua_rules() {
        config.push_rule(rule);
    }
    process(&resources, Options::new("src").with_configuration(config))
        .map_err(|err| Error::Transpile(err.to_string()))?;
    resources
        .get(&file)
        .map_err(|err| Error::Resource(format!("{err:?}")))
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
}
