use mlua_extras::typed::generator::{Definition, Definitions};
use mlua_extras::typed::{Type, TypedClassBuilder};

pub struct LuxDefinition {
    pub name: &'static str,
    pub build: fn() -> mlua_extras::typed::Type,
}

inventory::collect!(LuxDefinition);

pub fn definitions() -> Definitions {
    // Construct the Lux class manually, as mlua-extras has weird behaviour
    // where it silently skips certain UserData classes, which only negatively
    // affects us when generating the Lux class.
    let lux_class = TypedClassBuilder::default()
        .method::<(), ()>("loader", "Load the lux loader into the current Lua session")
        .field(
            "config",
            Type::named("config.Module"),
            "build::backend::Module for building a Lux `Config`",
        )
        .field(
            "workspace",
            Type::named("workspace.Module"),
            "build::backend::Module for interacting with a Lux workspace",
        )
        .field(
            "project",
            Type::named("project.Module"),
            "build::backend::Module for interacting with a Lux project",
        )
        .field(
            "operations",
            Type::named("operations.Module"),
            "build::backend::Module for Lux operations",
        )
        .field(
            "progress",
            Type::named("progress.Module"),
            "build::backend::Module for connecting Lux progress reports to the LSP server",
        )
        .build();

    let def = inventory::iter::<LuxDefinition>
        .into_iter()
        .filter(|item| item.name != "lux.Module")
        .fold(Definition::start(), |def, item| {
            def.register_as(item.name, (item.build)())
        })
        .register_as("lux.Module", Type::class(lux_class))
        .value::<crate::LuxModule>("lux")
        .finish();

    Definitions::start().define("lux", def).finish()
}
