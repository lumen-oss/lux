/// Specification for building a rock with the `rust-binary` build backend
#[derive(Debug, PartialEq, Default, Clone)]
pub struct RustBinaryBuildSpec {
    /// The name of the binary (or crate) to install, optionally including
    /// a version specifier (e.g. `foo@1.0.0`).
    pub binary: String,
    /// Cargo features to enable when building.
    pub features: Vec<String>,
}
