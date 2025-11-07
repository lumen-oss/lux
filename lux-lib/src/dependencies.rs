use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use which::which;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub name: String,
    pub found: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyReport {
    pub c_compiler: DependencyStatus,
    pub make: DependencyStatus,
    pub cmake: DependencyStatus,
    pub cargo: DependencyStatus,
    pub pkg_config: DependencyStatus,
}

impl DependencyReport {
    pub fn generate() -> Self {
        Self {
            c_compiler: check_c_compiler(),
            make: check_executable("make"),
            cmake: check_executable("cmake"),
            cargo: check_executable("cargo"),
            pkg_config: check_executable("pkg-config"),
        }
    }
}

fn check_executable(name: &str) -> DependencyStatus {
    match which(name) {
        Ok(path) => DependencyStatus {
            name: name.to_string(),
            found: true,
            path: Some(path),
            version: None,
        },
        Err(_) => DependencyStatus {
            name: name.to_string(),
            found: false,
            path: None,
            version: None,
        },
    }
}

fn check_c_compiler() -> DependencyStatus {
    // Try common C compilers in order of preference
    let compilers = vec!["cc", "gcc", "clang"];

    for compiler in compilers {
        if let Ok(path) = which(compiler) {
            return DependencyStatus {
                name: format!("C compiler ({})", compiler),
                found: true,
                path: Some(path),
                version: None,
            };
        }
    }

    DependencyStatus {
        name: "C compiler".to_string(),
        found: false,
        path: None,
        version: None,
    }
}
