use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use which::which;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    path: PathBuf,
    version: Option<String>,
}

impl ToolInfo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    name: String,
    info: Option<ToolInfo>,
}

impl Tool {
    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn info(&self) -> Option<&ToolInfo> {
        self.info.as_ref()
    }

    pub fn is_found(&self) -> bool {
        self.info.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainReport {
    c_compiler: Tool,
    make: Tool,
    cmake: Tool,
    cargo: Tool,
    pkg_config: Tool,
}

impl ToolchainReport {
    pub fn c_compiler(&self) -> &Tool {
        &self.c_compiler
    }
    pub fn make(&self) -> &Tool {
        &self.make
    }
    pub fn cmake(&self) -> &Tool {
        &self.cmake
    }
    pub fn cargo(&self) -> &Tool {
        &self.cargo
    }
    pub fn pkg_config(&self) -> &Tool {
        &self.pkg_config
    }

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

fn check_executable(name: &str) -> Tool {
    match which(name) {
        Ok(path) => {
            let version = try_get_version(Command::new(&path));

            Tool {
                name: name.to_string(),
                info: Some(ToolInfo { path, version }),
            }
        }
        Err(_) => Tool {
            name: name.to_string(),
            info: None,
        },
    }
}

fn check_c_compiler() -> Tool {
    match cc::Build::new().try_get_compiler() {
        Ok(compiler) => {
            let path = compiler.path().to_path_buf();

            let binary = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let name = format!("C compiler ({})", binary);
            let version = try_get_version(Command::new(&path));

            Tool {
                name,
                info: Some(ToolInfo { path, version }),
            }
        }

        Err(_) => Tool {
            name: "C compiler".to_string(),
            info: None,
        },
    }
}

fn try_get_version(mut cmd: Command) -> Option<String> {
    cmd.arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .and_then(|stdout| try_parse_version(&stdout))
}

fn try_parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_parse_version() {
        let gcc_output = "gcc (Ubuntu 11.4.0-1ubuntu1~22.04) 11.4.0\nCopyright (C) 2021 Free Software Foundation, Inc.\nThis is free software...";
        assert_eq!(
            try_parse_version(gcc_output),
            Some("gcc (Ubuntu 11.4.0-1ubuntu1~22.04) 11.4.0".to_string())
        );

        let loose_output = "\n\n   cmake version 3.22.1   \nConfigured safely";
        assert_eq!(
            try_parse_version(loose_output),
            Some("cmake version 3.22.1".to_string())
        );

        assert_eq!(try_parse_version("   \n\n  "), None);
    }

    #[test]
    fn test_live_environment_smoke() {
        let report = ToolchainReport::generate();

        let tools = [
            report.c_compiler(),
            report.make(),
            report.cmake(),
            report.cargo(),
            report.pkg_config(),
        ];

        for tool in tools {
            if let Some(info) = tool.info() {
                assert!(!info.path().as_os_str().is_empty());
            }
        }
    }
}
