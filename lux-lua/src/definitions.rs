use mlua_extras::typed::generator::{Definition, Definitions};

pub struct LuxDefinition {
    pub name: &'static str,
    pub build: fn() -> mlua_extras::typed::Type,
}

inventory::collect!(LuxDefinition);

pub fn definitions() -> Definitions {
    let def = inventory::iter::<LuxDefinition>
        .into_iter()
        .fold(Definition::start(), |def, item| {
            def.register_as(item.name, (item.build)())
        })
        .value::<crate::LuxModule>("lux")
        .finish();

    Definitions::start().define("lux", def).finish()
}
