use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, PartialEq, Default, Clone)]
pub struct RustMluaBuildSpec {
    /// Keys are module names in the format normally used by the `require()` function.
    /// values are the library names in the target directory (without the `lib` prefix).
    pub modules: HashMap<String, PathBuf>,
    /// Set if the cargo `target` directory is not in the source root.
    pub target_path: PathBuf,
    /// If set to `false` pass `--no-default-features` to cargo.
    pub default_features: bool,
    /// Pass additional features
    pub features: Vec<String>,
    /// Additional flags to be passed in the cargo invocation.
    pub cargo_extra_args: Vec<String>,
    /// Copy additional files to the `lua` directory.
    /// Keys are the sources, values the destinations (relative to the `lua` directory).
    pub include: HashMap<PathBuf, PathBuf>,
}

/*
impl UserData for RustMluaBuildSpec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("modules", |_, this, _: ()| Ok(this.modules.clone()));
        methods.add_method("target_path", |_, this, _: ()| Ok(this.target_path.clone()));
        methods.add_method("default_features", |_, this, _: ()| {
            Ok(this.default_features)
        });
        methods.add_method("features", |_, this, _: ()| Ok(this.features.clone()));
        methods.add_method("cargo_extra_args", |_, this, _: ()| {
            Ok(this.cargo_extra_args.clone())
        });
        methods.add_method("include", |_, this, _: ()| Ok(this.include.clone()));
    }
}
*/
