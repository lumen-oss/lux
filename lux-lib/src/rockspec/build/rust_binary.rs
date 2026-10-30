/// Specification for building a rock with the `rust-binary` build backend
#[derive(Debug, PartialEq, Default, Clone)]
pub struct RustBinaryBuildSpec {
    /// The name of the crate to install binaries from.
    /// Must be specified in multi-package workspaces.
    pub package: Option<String>,
    /// Cargo features to enable when building.
    pub features: Vec<String>,
}
