use std::{collections::HashMap, path::PathBuf};

use serde::Serialize;

#[cfg(not(target_env = "msvc"))]
const MAKEFILE: &str = "Makefile";
#[cfg(target_env = "msvc")]
const MAKEFILE: &str = "Makefile.win";

#[derive(Debug, PartialEq, Clone, Serialize)]
pub struct MakeBuildSpec {
    /// Makefile to be used.
    /// Default is "Makefile" on Unix variants and "Makefile.win" under Win32.
    pub makefile: PathBuf,
    pub build_target: Option<String>,
    /// Whether to perform a make pass on the target indicated by `build_target`.
    /// Default is true (i.e., to run make).
    pub build_pass: bool,
    /// Default is "install"
    pub install_target: String,
    /// Whether to perform a make pass on the target indicated by `install_target`.
    /// Default is true (i.e., to run make).
    pub install_pass: bool,
    /// Assignments to be passed to make during the build pass
    pub build_variables: HashMap<String, String>,
    /// Assignments to be passed to make during the install pass
    pub install_variables: HashMap<String, String>,
    /// Assignments to be passed to make during both passes
    pub variables: HashMap<String, String>,
}

/*
impl UserData for MakeBuildSpec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("makefile", |_, this, _: ()| Ok(this.makefile.clone()));
        methods.add_method("build_target", |_, this, _: ()| {
            Ok(this.build_target.clone())
        });
        methods.add_method("build_pass", |_, this, _: ()| Ok(this.build_pass));
        methods.add_method("install_target", |_, this, _: ()| {
            Ok(this.install_target.clone())
        });
        methods.add_method("install_pass", |_, this, _: ()| Ok(this.install_pass));
        methods.add_method("build_variables", |_, this, _: ()| {
            Ok(this.build_variables.clone())
        });
        methods.add_method("install_variables", |_, this, _: ()| {
            Ok(this.install_variables.clone())
        });
        methods.add_method("variables", |_, this, _: ()| Ok(this.variables.clone()));
    }
}
*/

impl Default for MakeBuildSpec {
    fn default() -> Self {
        Self {
            makefile: default_makefile_name(),
            build_target: Option::default(),
            build_pass: default_pass(),
            install_target: default_install_target(),
            install_pass: default_pass(),
            build_variables: HashMap::default(),
            install_variables: HashMap::default(),
            variables: HashMap::default(),
        }
    }
}

fn default_makefile_name() -> PathBuf {
    PathBuf::from(MAKEFILE)
}

fn default_pass() -> bool {
    true
}

fn default_install_target() -> String {
    "install".into()
}
