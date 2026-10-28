use std::path::PathBuf;

use lux_lib::{
    lockfile::{Lockfile, OptState, PinnedState},
    lua_rockspec::RockSourceSpec,
};
use lux_lua::lua_impls::{AddDependencyLua, LockfileReadOnlyLua, LuaDependencySpecLua};
use mlua::{IntoLua, Lua};

#[test]
fn lua_dynamic_dependency_spec() {
    let lua = Lua::new();
    let spec: LuaDependencySpecLua = lua
        .load(
            r#"return {
                name = "foo",
                version = "1.2.3",
                opt = true,
                pin = true,
                git = "https://github.com/foo/bar",
                rev = "v1.2.3",
            }"#,
        )
        .eval()
        .unwrap();

    assert_eq!(spec.0.name().to_string(), "foo");
    assert_eq!(*spec.0.opt(), OptState::Optional);
    assert_eq!(*spec.0.pin(), PinnedState::Pinned);
    assert!(matches!(spec.0.source(), Some(RockSourceSpec::Git(_))));
}

#[test]
fn add_dependency_parses_packages_and_git_shorthands() {
    let lua = Lua::new();

    let package: AddDependencyLua = lua.load(r#"return "foo >= 1.0""#).eval().unwrap();
    assert!(matches!(package, AddDependencyLua::Req(_)));

    let git: AddDependencyLua = lua.load(r#"return "github:foo/bar""#).eval().unwrap();
    assert!(matches!(git, AddDependencyLua::GitShorthand(_)));
}

#[test]
fn lockfile_exposes_entrypoints_and_metadata() {
    let lockfile = Lockfile::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../lux-lib/resources/test/sample-tree/5.1/lux.lock"),
    )
    .unwrap();
    let lua = Lua::new();
    let lockfile = LockfileReadOnlyLua(lockfile).into_lua(&lua).unwrap();
    lua.globals().set("lockfile", lockfile).unwrap();

    lua.load(
        r#"
        local entrypoints = lockfile:entrypoints()
        assert(next(entrypoints) ~= nil, "expected at least one entrypoint")

        for name, package in pairs(entrypoints) do
            local id = package:id()
            assert(lockfile:is_entrypoint(id))
            assert(package:name() == name)
            assert(type(package:opt()) == "boolean")
        end

        assert(#lockfile:reachable_rocks() > 0)
        "#,
    )
    .exec()
    .unwrap();
}
