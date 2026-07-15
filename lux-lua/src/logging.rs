use mlua::prelude::*;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData, WrappedBuilder};

#[derive(Clone)]
pub(crate) struct LoggingModule;

impl Typed for LoggingModule {
    fn ty() -> Type {
        Type::named("LoggingModule")
    }
}

impl TypedUserData for LoggingModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Toggle logging capture");
        methods.param(
            "enabled",
            "Whether logging capture should be enabled (default: false)",
        );
        methods.add_function("set_enabled", |_, enabled: bool| {
            let state = if enabled {
                lux_lib::logging::LoggingState::Enabled
            } else {
                lux_lib::logging::LoggingState::Disabled
            };
            lux_lib::logging::set_state(state);
            Ok(())
        });

        methods.document("Drain all buffered log entries and return them as an array of tables");
        methods.ret(
            "Array of log entries, each with 'level', 'message', and optional 'target' fields",
        );
        methods.add_function("drain", |lua, ()| {
            let entries = lux_lib::logging::drain();
            lua.to_value(&entries).map_err(mlua::Error::external)
        });

        methods.document("Clear all buffered log entries without returning them");
        methods.add_function("clear", |_, ()| {
            lux_lib::logging::clear();
            Ok(())
        });
    }

    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for capturing Lux log output");
    }
}

impl mlua::UserData for LoggingModule {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "LoggingModule",
            build: || Type::class(TypedClassBuilder::new::<super::LoggingModule>().build()),
        }
    }
}
