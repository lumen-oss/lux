use std::collections::HashMap;

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

fn default_pass() -> bool {
    true
}
