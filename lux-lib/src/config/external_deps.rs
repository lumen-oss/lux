use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Used as a fallback when searching for external dependencies if they
/// cannot be found using pkg-config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExternalDependencySearchConfig {
    /// Patterns for binary files
    #[serde(default = "default_bin_patterns")]
    pub(crate) bin_patterns: Vec<String>,
    /// Patterns for header files
    #[serde(default = "default_include_patterns")]
    pub(crate) include_patterns: Vec<String>,
    /// Patterns for library files
    #[serde(default = "default_lib_patterns")]
    pub(crate) lib_patterns: Vec<String>,
    /// Default binary subdirectory
    #[serde(default = "default_bin_subdir")]
    pub(crate) bin_subdir: String,
    /// Default include subdirectory
    #[serde(default = "default_include_subdir")]
    pub(crate) include_subdir: String,
    /// Default library subdirectory
    #[serde(default = "default_lib_subdirs")]
    pub(crate) lib_subdirs: Vec<PathBuf>,
    /// System-wide search paths
    #[serde(default = "default_prefixes")]
    pub(crate) search_prefixes: Vec<PathBuf>,
    /// Known installation prefixes for specific dependencies.
    /// These can also be set via environment variables.
    pub(crate) prefixes: HashMap<String, PathBuf>,
}

impl Default for ExternalDependencySearchConfig {
    fn default() -> Self {
        Self {
            bin_patterns: default_bin_patterns(),
            include_patterns: default_include_patterns(),
            lib_patterns: default_lib_patterns(),
            bin_subdir: default_bin_subdir(),
            include_subdir: default_include_subdir(),
            lib_subdirs: default_lib_subdirs(),
            search_prefixes: default_prefixes(),
            prefixes: HashMap::default(),
        }
    }
}

fn default_bin_patterns() -> Vec<String> {
    vec!["?".into()]
}

fn default_include_patterns() -> Vec<String> {
    vec!["?.h".into()]
}

#[cfg(target_family = "unix")]
fn default_lib_patterns() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        vec!["lib?.dylib".to_string(), "lib?.a".to_string()]
    }

    #[cfg(not(target_os = "macos"))]
    {
        vec!["lib?.so".to_string(), "lib?.a".to_string()]
    }
}

#[cfg(target_family = "windows")]
fn default_lib_patterns() -> Vec<String> {
    vec!["?.dll".to_string(), "?.lib".to_string()]
}

#[cfg(target_family = "unix")]
fn default_prefixes() -> Vec<PathBuf> {
    use std::path::PathBuf;

    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/usr"),
            PathBuf::from("/usr/local"),
            PathBuf::from("/opt/local"),
            PathBuf::from("/opt/homebrew"),
            PathBuf::from("/opt"),
        ]
    }

    #[cfg(not(target_os = "macos"))]
    {
        vec![
            PathBuf::from("/usr"),
            PathBuf::from("/usr/local"),
            PathBuf::from("/opt/local"),
            PathBuf::from("/opt"),
        ]
    }
}

#[cfg(target_family = "windows")]
fn default_prefixes() -> Vec<PathBuf> {
    vec![
        PathBuf::from(r"C:\Program Files"),
        PathBuf::from(r"C:\Program Files (x86)"),
    ]
}

fn default_bin_subdir() -> String {
    "bin".into()
}

fn default_include_subdir() -> String {
    "include".into()
}

fn default_lib_subdirs() -> Vec<PathBuf> {
    vec!["lib".into(), "lib64".into()]
}
