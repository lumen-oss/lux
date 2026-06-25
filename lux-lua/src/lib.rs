#![cfg_attr(feature = "test", allow(unused_imports, dead_code))]

use mlua::prelude::*;
use mlua_extras::typed::{Type, Typed, TypedDataFields, TypedDataMethods, TypedUserData};

mod config;
#[cfg(feature = "definitions")]
pub mod definitions;
mod loader;
pub mod lua_impls;
mod operations;
mod project;
mod workspace;

#[derive(Clone)]
pub(crate) struct LuxModule;

impl Typed for LuxModule {
    fn ty() -> Type {
        Type::named("LuxModule")
    }
}

impl TypedUserData for LuxModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_function("loader", |lua, ()| loader::load_loader(lua));
    }

    fn add_fields<F: TypedDataFields<Self>>(fields: &mut F) {
        fields.add_field("config", config::ConfigModule);
        fields.add_field("workspace", workspace::WorkspaceModule);
        fields.add_field("project", project::ProjectModule);
        fields.add_field("operations", operations::OperationsModule);
    }
}

impl mlua::UserData for LuxModule {
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

    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "LuxModule",
            build: || Type::class(TypedClassBuilder::new::<super::LuxModule>().build()),
        }
    }
}

#[cfg_attr(not(feature = "test"), mlua::lua_module)]
fn lux(lua: &Lua) -> LuaResult<LuaAnyUserData> {
    #[cfg(not(any(
        feature = "lua51",
        feature = "lua52",
        feature = "lua53",
        feature = "lua54",
        feature = "lua55",
        feature = "luajit",
        feature = "test"
    )))]
    compile_error!(
        "
        At least one Lua version feature must be enabled. \
        Please enable one of the following features: \
        lua51, lua52, lua53, lua54, lua55, luajit."
    );

    lua.create_userdata(LuxModule)
}
