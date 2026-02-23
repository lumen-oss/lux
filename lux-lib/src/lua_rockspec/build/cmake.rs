use std::collections::HashMap;

use mlua::UserData;

#[derive(Debug, PartialEq, Clone)]
pub struct CMakeBuildSpec {
    pub cmake_lists_content: Option<String>,
    /// Whether to perform a build pass.
    /// Default is true.
    pub build_pass: bool,
    /// Whether to perform an install pass.
    /// Default is true.
    pub install_pass: bool,
    pub variables: HashMap<String, String>,
}

impl Default for CMakeBuildSpec {
    fn default() -> Self {
        Self {
            cmake_lists_content: Default::default(),
            build_pass: default_pass(),
            install_pass: default_pass(),
            variables: Default::default(),
        }
    }
}

/*
impl UserData for CMakeBuildSpec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cmake_lists_content", |_, this, _: ()| {
            Ok(this.cmake_lists_content.clone())
        });
        methods.add_method("build_pass", |_, this, _: ()| Ok(this.build_pass));
        methods.add_method("install_pass", |_, this, _: ()| Ok(this.install_pass));
        methods.add_method("variables", |_, this, _: ()| Ok(this.variables.clone()));
    }
}
*/

fn default_pass() -> bool {
    true
}
