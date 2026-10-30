use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FilterSpec {
    /// Include Rockspec
    pub rockspec: bool,
    /// Include Src
    pub src: bool,
    /// Include Binary
    pub binary: bool,
}

impl Default for FilterSpec {
    fn default() -> Self {
        Self {
            rockspec: true,
            src: true,
            binary: true,
        }
    }
}
