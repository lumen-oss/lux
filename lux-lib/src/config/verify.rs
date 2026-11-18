use serde::{Deserialize, Serialize};

/// Whether and how to verify signatures of downloaded rockspecs and rocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "lowercase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Do not verify signatures.
    #[default]
    Off,
    /// Verify signatures and warn on failure.
    Warn,
    /// Verify signatures and fail on failure.
    Require,
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Require => "require",
        }
    }

    pub(crate) fn allows_binary_rocks(self) -> bool {
        !matches!(self, Self::Require)
    }

    pub(crate) fn restrict_package_types(
        self,
        package_types: &mut crate::remote::package::kind::FilterSpec,
    ) {
        if !self.allows_binary_rocks() {
            tracing::warn!("excluding binary rocks because signature verification is required");
            package_types.binary = false;
        }
    }
}

impl std::str::FromStr for Policy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "warn" => Ok(Self::Warn),
            "require" => Ok(Self::Require),
            _ => Err(value.to_string()),
        }
    }
}
