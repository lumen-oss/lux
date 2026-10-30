use serde::Deserialize;
use url::Url;

/// A rock's metadata, to be displayed on the remote package server
#[derive(Clone, Deserialize, Debug, PartialEq, Default, lux_macros::DisplayAsLuaKV)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[display_lua(key = "description")]
pub struct RockDescription {
    /// A one-line description of the package.
    pub summary: Option<String>,
    /// A longer description of the package.
    pub detailed: Option<String>,
    /// The license used by the package.
    pub license: Option<String>,
    /// An URL for the project. This is not the URL for the tarball, but the address of a website.
    #[serde(default, deserialize_with = "deserialize_url")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub homepage: Option<Url>,
    /// An URL for the project's issue tracker.
    #[serde(default, deserialize_with = "deserialize_url")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub issues_url: Option<Url>,
    /// Contact information for the rockspec maintainer.
    pub maintainer: Option<String>,
    /// A list of short strings that specify labels for categorization of this rock.
    #[serde(default)]
    pub labels: Vec<String>,
}

fn deserialize_url<'de, D>(deserializer: D) -> Result<Option<Url>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    s.map(|s| Url::parse(&s).map_err(serde::de::Error::custom))
        .transpose()
}
