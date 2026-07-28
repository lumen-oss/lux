use std::sync::Once;

use lux_lib::progress;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::lua_impls::WorkspaceLua;

#[derive(Clone)]
pub(crate) struct ProgressModule;

impl Typed for ProgressModule {
    fn ty() -> Type {
        Type::named("ProgressModule")
    }
}

impl TypedUserData for ProgressModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Connect progress reports to a running LSP server for a given workspace");
        methods.param(
            "workspace",
            "The workspace to connect progress reporting to",
        );
        methods.add_function("set_connection", |_, workspace: WorkspaceLua| {
            static INIT: Once = Once::new();
            INIT.call_once(|| {
                let _ = tracing_subscriber::registry()
                    .with(lux_lib::progress::progress_layer())
                    .try_init();
            });

            progress::set_connection(&workspace.0);

            Ok(())
        });
    }

    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for connecting Lux progress reports to a running LSP server");
    }
}

impl mlua::UserData for ProgressModule {
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

    use super::ProgressModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "ProgressModule",
            build: || Type::class(TypedClassBuilder::new::<ProgressModule>().build()),
        }
    }
}
