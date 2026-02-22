use itertools::Itertools;
use mlua::{FromLua, IntoLuaMulti, Lua, LuaSerdeExt, UserData, Value};
use std::{cmp::Ordering, collections::HashMap, marker::PhantomData};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use thiserror::Error;

use serde::{
    de::{self, DeserializeOwned},
    Deserialize, Deserializer,
};
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};

use super::{DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue};

/// Identifier by a platform.
/// The `PartialOrd` instance views more specific platforms as `Greater`
#[derive(Deserialize_enum_str, Serialize_enum_str, PartialEq, Eq, Hash, Debug, Clone, EnumIter)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum PlatformIdentifier {
    // TODO: Add undocumented platform identifiers from luarocks codebase?
    Unix,
    Windows,
    Win32,
    Cygwin,
    MacOSX,
    Linux,
    FreeBSD,
    #[serde(other)]
    Unknown(String),
}

impl Default for PlatformIdentifier {
    fn default() -> Self {
        target_identifier()
    }
}

// Order by specificity -> less specific = `Less`
impl PartialOrd for PlatformIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (PlatformIdentifier::Unix, PlatformIdentifier::Cygwin) => Some(Ordering::Less),
            (PlatformIdentifier::Unix, PlatformIdentifier::MacOSX) => Some(Ordering::Less),
            (PlatformIdentifier::Unix, PlatformIdentifier::Linux) => Some(Ordering::Less),
            (PlatformIdentifier::Unix, PlatformIdentifier::FreeBSD) => Some(Ordering::Less),
            (PlatformIdentifier::Windows, PlatformIdentifier::Win32) => Some(Ordering::Greater),
            (PlatformIdentifier::Win32, PlatformIdentifier::Windows) => Some(Ordering::Less),
            (PlatformIdentifier::Cygwin, PlatformIdentifier::Unix) => Some(Ordering::Greater),
            (PlatformIdentifier::MacOSX, PlatformIdentifier::Unix) => Some(Ordering::Greater),
            (PlatformIdentifier::Linux, PlatformIdentifier::Unix) => Some(Ordering::Greater),
            (PlatformIdentifier::FreeBSD, PlatformIdentifier::Unix) => Some(Ordering::Greater),
            _ if self == other => Some(Ordering::Equal),
            _ => None,
        }
    }
}

impl FromLua for PlatformIdentifier {
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        let string = String::from_lua(value, lua)?;
        Ok(string
            .parse()
            .unwrap_or(PlatformIdentifier::Unknown(string)))
    }
}

/// Retrieves the platform identifier for the target platform
///
/// NOTE: This is the platform lux was built with.
/// As we don't support cross-compiling luarocks packages, we currently expect
/// users to use a version of lux that was built with the same platform
/// as the one they are targeting
fn target_identifier() -> PlatformIdentifier {
    if cfg!(target_env = "msvc") {
        PlatformIdentifier::Windows
    } else if cfg!(target_os = "linux") {
        PlatformIdentifier::Linux
    } else if cfg!(target_os = "macos") || cfg!(target_vendor = "apple") {
        PlatformIdentifier::MacOSX
    } else if cfg!(target_os = "freebsd") {
        PlatformIdentifier::FreeBSD
    } else if which::which("cygpath").is_ok() {
        PlatformIdentifier::Cygwin
    } else {
        PlatformIdentifier::Unix
    }
}

impl PlatformIdentifier {
    /// Get identifiers that are a subset of this identifier.
    /// For example, Unix is a subset of Linux
    pub fn get_subsets(&self) -> Vec<Self> {
        PlatformIdentifier::iter()
            .filter(|identifier| identifier.is_subset_of(self))
            .collect()
    }

    /// Get identifiers that are an extension of this identifier.
    /// For example, Linux is an extension of Unix
    pub fn get_extended_platforms(&self) -> Vec<Self> {
        PlatformIdentifier::iter()
            .filter(|identifier| identifier.is_extension_of(self))
            .collect()
    }

    /// e.g. Unix is a subset of Linux
    fn is_subset_of(&self, other: &PlatformIdentifier) -> bool {
        self.partial_cmp(other) == Some(Ordering::Less)
    }

    /// e.g. Linux is an extension of Unix
    fn is_extension_of(&self, other: &PlatformIdentifier) -> bool {
        self.partial_cmp(other) == Some(Ordering::Greater)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformSupport {
    /// Do not match this platform
    platform_map: HashMap<PlatformIdentifier, bool>,
}

impl Default for PlatformSupport {
    fn default() -> Self {
        Self {
            platform_map: PlatformIdentifier::iter()
                .filter(|identifier| !matches!(identifier, PlatformIdentifier::Unknown(_)))
                .map(|identifier| (identifier, true))
                .collect(),
        }
    }
}

impl UserData for PlatformSupport {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_supported", |_, this, platform: PlatformIdentifier| {
            Ok(this.is_supported(&platform))
        });
    }
}

impl<'de> Deserialize<'de> for PlatformSupport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let platforms: Vec<String> = Vec::deserialize(deserializer)?;
        Self::parse(&platforms).map_err(de::Error::custom)
    }
}

impl DisplayAsLuaKV for PlatformSupport {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "supported_platforms".to_string(),
            value: DisplayLuaValue::List(
                self.platforms()
                    .iter()
                    .map(|(platform, supported)| {
                        DisplayLuaValue::String(format!(
                            "{}{}",
                            if *supported { "" } else { "!" },
                            platform,
                        ))
                    })
                    .collect(),
            ),
        }
    }
}

#[derive(Error, Debug)]
pub enum PlatformValidationError {
    #[error("error when parsing platform identifier: {0}")]
    ParseError(String),

    #[error("conflicting supported platform entries")]
    ConflictingEntries,
}

impl PlatformSupport {
    fn validate_platforms(
        platforms: &[String],
    ) -> Result<HashMap<PlatformIdentifier, bool>, PlatformValidationError> {
        platforms
            .iter()
            .try_fold(HashMap::new(), |mut platforms, platform| {
                // Platform assertions can exist in one of the following forms:
                // - `platform` - a positive assertion for the platform (the platform must be present)
                // - `!platform` - a negative assertion for the platform (any platform *but* this one must be present)
                let (is_positive_assertion, platform) = platform
                    .strip_prefix('!')
                    .map(|str| (false, str))
                    .unwrap_or((true, platform));

                let platform_identifier = platform
                    .parse::<PlatformIdentifier>()
                    .map_err(|err| PlatformValidationError::ParseError(err.to_string()))?;

                // If a platform with the same name exists already and is contradictory
                // then throw an error. An example of such a contradiction is e.g.:
                // [`win32`, `!win32`]
                if platforms
                    .get(&platform_identifier)
                    .unwrap_or(&is_positive_assertion)
                    != &is_positive_assertion
                {
                    return Err(PlatformValidationError::ConflictingEntries);
                }

                platforms.insert(platform_identifier.clone(), is_positive_assertion);

                let subset_or_extended_platforms = if is_positive_assertion {
                    platform_identifier.get_extended_platforms()
                } else {
                    platform_identifier.get_subsets()
                };

                for sub_platform in subset_or_extended_platforms {
                    if platforms
                        .get(&sub_platform)
                        .unwrap_or(&is_positive_assertion)
                        != &is_positive_assertion
                    {
                        // TODO(vhyrro): More detailed errors
                        return Err(PlatformValidationError::ConflictingEntries);
                    }

                    platforms.insert(sub_platform, is_positive_assertion);
                }

                Ok(platforms)
            })
    }

    pub fn parse(platforms: &[String]) -> Result<Self, PlatformValidationError> {
        // Platforms are matched in one of two ways: exclusively or inclusively.
        // If only positive matches are present, then the platforms are matched inclusively (as you only support the matches that you specified).
        // If any negative matches are present, then the platforms are matched exclusively (as you want to support any operating system *other* than the ones you negated).
        match platforms {
            [] => Ok(Self::default()),
            platforms if platforms.iter().any(|platform| platform.starts_with('!')) => {
                let mut platform_map = Self::validate_platforms(platforms)?;

                // Loop through all identifiers and set them to true if they are not present in
                // the map (exclusive matching).
                for identifier in PlatformIdentifier::iter() {
                    if !matches!(identifier, PlatformIdentifier::Unknown(_)) {
                        platform_map.entry(identifier).or_insert(true);
                    }
                }

                Ok(Self { platform_map })
            }
            // Only validate positive matches (inclusive matching)
            platforms => Ok(Self {
                platform_map: Self::validate_platforms(platforms)?,
            }),
        }
    }

    pub fn is_supported(&self, platform: &PlatformIdentifier) -> bool {
        self.platform_map.get(platform).cloned().unwrap_or(false)
    }

    pub(crate) fn platforms(&self) -> &HashMap<PlatformIdentifier, bool> {
        &self.platform_map
    }
}

pub trait PartialOverride: Sized {
    type Err: std::error::Error;

    fn apply_overrides(&self, override_val: &Self) -> Result<Self, Self::Err>;
}

pub trait PlatformOverridable: PartialOverride {
    type Err: std::error::Error;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default;
}

/// Data that that can vary per platform
#[derive(Clone, Debug, PartialEq)]
pub struct PerPlatform<T> {
    /// The base data, applicable if no platform is specified
    pub(crate) default: T,
    /// The per-platform override, if present.
    pub(crate) per_platform: HashMap<PlatformIdentifier, T>,
}

impl<T> PerPlatform<T> {
    pub(crate) fn new(default: T) -> Self {
        Self {
            default,
            per_platform: HashMap::default(),
        }
    }

    /// Merge per-platform overrides for the configured build target platform,
    /// with more specific platform overrides having higher priority.
    pub fn current_platform(&self) -> &T {
        self.for_platform_identifier(&target_identifier())
    }

    fn for_platform_identifier(&self, identifier: &PlatformIdentifier) -> &T {
        self.get(identifier)
    }

    pub fn get(&self, platform: &PlatformIdentifier) -> &T {
        self.per_platform.get(platform).unwrap_or(
            platform
                .get_subsets()
                .into_iter()
                // More specific platforms first.
                // This is safe because a platform's subsets
                // can be totally ordered among each other.
                .sorted_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal))
                .find(|identifier| self.per_platform.contains_key(identifier))
                .and_then(|identifier| self.per_platform.get(&identifier))
                .unwrap_or(&self.default),
        )
    }

    pub(crate) fn map<U, F>(&self, cb: F) -> PerPlatform<U>
    where
        F: Fn(&T) -> U,
    {
        PerPlatform {
            default: cb(&self.default),
            per_platform: self
                .per_platform
                .iter()
                .map(|(identifier, value)| (identifier.clone(), cb(value)))
                .collect(),
        }
    }
}

impl<U, E> PerPlatform<Result<U, E>>
where
    E: std::error::Error,
{
    pub fn transpose(self) -> Result<PerPlatform<U>, E> {
        Ok(PerPlatform {
            default: self.default?,
            per_platform: self
                .per_platform
                .into_iter()
                .map(|(identifier, value)| Ok((identifier, value?)))
                .try_collect()?,
        })
    }
}

impl<T: Default> Default for PerPlatform<T> {
    fn default() -> Self {
        Self {
            default: T::default(),
            per_platform: HashMap::default(),
        }
    }
}

impl<'de, T> Deserialize<'de> for PerPlatform<T>
where
    T: Deserialize<'de>,
    T: Clone,
    T: PartialOverride,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = toml::map::Map::deserialize(deserializer)?;

        let mut per_platform: HashMap<PlatformIdentifier, T> = map
            .remove("platforms")
            .map_or(Ok(HashMap::default()), |platforms| platforms.try_into())
            .map_err(serde::de::Error::custom)?;

        let default: T = map.try_into().map_err(serde::de::Error::custom)?;

        apply_per_platform_overrides(&mut per_platform, &default)
            .map_err(serde::de::Error::custom)?;

        Ok(PerPlatform {
            default,
            per_platform,
        })
    }
}

impl<T> FromLua for PerPlatform<T>
where
    T: PlatformOverridable,
    T: PartialOverride,
    T: DeserializeOwned,
    T: Default,
    T: Clone,
{
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        match &value {
            list @ Value::Table(tbl) => {
                let mut per_platform = match tbl.get("platforms")? {
                    val @ Value::Table(_) => Ok(lua.from_value(val)?),
                    Value::Nil => Ok(HashMap::default()),
                    val => Err(mlua::Error::DeserializeError(format!(
                        "Expected platforms to be a table or nil, but got {}",
                        val.type_name()
                    ))),
                }?;
                let _ = tbl.raw_remove("platforms");
                let default = lua.from_value(list.to_owned())?;
                apply_per_platform_overrides(&mut per_platform, &default).map_err(
                    |err: <T as PartialOverride>::Err| {
                        mlua::Error::DeserializeError(err.to_string())
                    },
                )?;
                Ok(PerPlatform {
                    default,
                    per_platform,
                })
            }
            Value::Nil => T::on_nil().map_err(|err| mlua::Error::DeserializeError(err.to_string())),
            val => Err(mlua::Error::DeserializeError(format!(
                "Expected rockspec external dependencies to be a table or nil, but got {}",
                val.type_name()
            ))),
        }
    }
}

impl<T> UserData for PerPlatform<T>
where
    T: IntoLuaMulti + Clone,
{
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // TODO(mrcjkb): YAGNI?
        // methods.add_method("current_platform", |_, this, _: ()| {
        //     Ok(this.for_target_platform().clone())
        // });
        methods.add_method("get", |_, this, platform: PlatformIdentifier| {
            Ok(this.get(&platform).clone())
        });
    }
}

/// Newtype wrapper used to implement a `FromLua` instance for types using `TryFrom`
/// This is necessary, because Rust doesn't yet support specialization.
pub struct PerPlatformWrapper<T, G> {
    pub un_per_platform: PerPlatform<T>,
    phantom: PhantomData<G>,
}

impl<T, G> FromLua for PerPlatformWrapper<T, G>
where
    T: TryFrom<G, Error: ToString>,
    G: PlatformOverridable<Err: ToString>,
    G: DeserializeOwned,
    G: Default,
    G: Clone,
{
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        let internal = PerPlatform::from_lua(value, lua)?;
        let per_platform: HashMap<_, _> = internal
            .per_platform
            .into_iter()
            .map(|(platform, internal_override)| {
                let override_spec = T::try_from(internal_override)
                    .map_err(|err| mlua::Error::DeserializeError(err.to_string()))?;

                Ok((platform, override_spec))
            })
            .try_collect::<_, _, mlua::Error>()?;
        let un_per_platform = PerPlatform {
            default: T::try_from(internal.default)
                .map_err(|err| mlua::Error::DeserializeError(err.to_string()))?,
            per_platform,
        };
        Ok(PerPlatformWrapper {
            un_per_platform,
            phantom: PhantomData,
        })
    }
}

impl<'de, T, G> Deserialize<'de> for PerPlatformWrapper<T, G>
where
    T: TryFrom<G, Error: std::fmt::Display>,
    G: PlatformOverridable<Err: ToString>,
    G: DeserializeOwned,
    G: Default,
    G: Clone,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let internal = PerPlatform::deserialize(deserializer)?;
        let per_platform: HashMap<_, _> = internal
            .per_platform
            .into_iter()
            .map(|(platform, internal_override)| {
                let override_spec =
                    T::try_from(internal_override).map_err(serde::de::Error::custom)?;

                Ok((platform, override_spec))
            })
            .try_collect::<_, _, D::Error>()?;
        let un_per_platform = PerPlatform {
            default: T::try_from(internal.default).map_err(serde::de::Error::custom)?,
            per_platform,
        };
        Ok(PerPlatformWrapper {
            un_per_platform,
            phantom: PhantomData,
        })
    }
}

fn apply_per_platform_overrides<T>(
    per_platform: &mut HashMap<PlatformIdentifier, T>,
    base: &T,
) -> Result<(), T::Err>
where
    T: PartialOverride,
    T: Clone,
{
    let per_platform_raw = per_platform.clone();
    for (platform, overrides) in per_platform.clone() {
        // Add base values for each platform
        let overridden = base.apply_overrides(&overrides)?;
        per_platform.insert(platform, overridden);
    }
    for (platform, overrides) in per_platform_raw {
        // Add extended platform dependencies (without base deps) for each platform
        for extended_platform in &platform.get_extended_platforms() {
            if let Some(extended_overrides) = per_platform.get(extended_platform) {
                per_platform.insert(
                    extended_platform.to_owned(),
                    extended_overrides.apply_overrides(&overrides)?,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use proptest::prelude::*;

    fn platform_identifier_strategy() -> impl Strategy<Value = PlatformIdentifier> {
        prop_oneof![
            Just(PlatformIdentifier::Unix),
            Just(PlatformIdentifier::Windows),
            Just(PlatformIdentifier::Win32),
            Just(PlatformIdentifier::Cygwin),
            Just(PlatformIdentifier::MacOSX),
            Just(PlatformIdentifier::Linux),
            Just(PlatformIdentifier::FreeBSD),
        ]
    }

    #[tokio::test]
    async fn sort_platform_identifier_more_specific_last() {
        let mut platforms = vec![
            PlatformIdentifier::Cygwin,
            PlatformIdentifier::Linux,
            PlatformIdentifier::Unix,
        ];
        platforms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        assert_eq!(
            platforms,
            vec![
                PlatformIdentifier::Unix,
                PlatformIdentifier::Cygwin,
                PlatformIdentifier::Linux
            ]
        );
        let mut platforms = vec![PlatformIdentifier::Windows, PlatformIdentifier::Win32];
        platforms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        assert_eq!(
            platforms,
            vec![PlatformIdentifier::Win32, PlatformIdentifier::Windows]
        )
    }

    #[tokio::test]
    async fn test_is_subset_of() {
        assert!(PlatformIdentifier::Unix.is_subset_of(&PlatformIdentifier::Linux));
        assert!(PlatformIdentifier::Unix.is_subset_of(&PlatformIdentifier::MacOSX));
        assert!(!PlatformIdentifier::Linux.is_subset_of(&PlatformIdentifier::Unix));
    }

    #[tokio::test]
    async fn test_is_extension_of() {
        assert!(PlatformIdentifier::Linux.is_extension_of(&PlatformIdentifier::Unix));
        assert!(PlatformIdentifier::MacOSX.is_extension_of(&PlatformIdentifier::Unix));
        assert!(!PlatformIdentifier::Unix.is_extension_of(&PlatformIdentifier::Linux));
    }

    #[tokio::test]
    async fn per_platform() {
        let foo = PerPlatform {
            default: "default",
            per_platform: vec![
                (PlatformIdentifier::Unix, "unix"),
                (PlatformIdentifier::FreeBSD, "freebsd"),
                (PlatformIdentifier::Cygwin, "cygwin"),
                (PlatformIdentifier::Linux, "linux"),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(*foo.get(&PlatformIdentifier::MacOSX), "unix");
        assert_eq!(*foo.get(&PlatformIdentifier::Linux), "linux");
        assert_eq!(*foo.get(&PlatformIdentifier::FreeBSD), "freebsd");
        assert_eq!(*foo.get(&PlatformIdentifier::Cygwin), "cygwin");
        assert_eq!(*foo.get(&PlatformIdentifier::Windows), "default");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_target_identifier() {
        run_test_target_identifier(PlatformIdentifier::Linux)
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_target_identifier() {
        run_test_target_identifier(PlatformIdentifier::MacOSX)
    }

    #[cfg(target_env = "msvc")]
    #[tokio::test]
    async fn test_target_identifier() {
        run_test_target_identifier(PlatformIdentifier::Windows)
    }

    #[cfg(target_os = "android")]
    #[tokio::test]
    async fn test_target_identifier() {
        run_test_target_identifier(PlatformIdentifier::Unix)
    }

    fn run_test_target_identifier(expected: PlatformIdentifier) {
        assert_eq!(expected, target_identifier());
    }

    proptest! {
        #[test]
        fn supported_platforms(identifier in platform_identifier_strategy()) {
            let identifier_str = identifier.to_string();
            let platforms = vec![identifier_str];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            prop_assert!(platform_support.is_supported(&identifier))
        }

        #[test]
        fn unsupported_platforms_only(unsupported in platform_identifier_strategy(), supported in platform_identifier_strategy()) {
            if supported == unsupported
                || unsupported.is_extension_of(&supported) {
                return Ok(());
            }
            let identifier_str = format!("!{unsupported}");
            let platforms = vec![identifier_str];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            prop_assert!(!platform_support.is_supported(&unsupported));
            prop_assert!(platform_support.is_supported(&supported))
        }

        #[test]
        fn supported_and_unsupported_platforms(unsupported in platform_identifier_strategy(), unspecified in platform_identifier_strategy()) {
            if unspecified == unsupported
                || unsupported.is_extension_of(&unspecified) {
                return Ok(());
            }
            let supported_str = unspecified.to_string();
            let unsupported_str = format!("!{unsupported}");
            let platforms = vec![supported_str, unsupported_str];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            prop_assert!(platform_support.is_supported(&unspecified));
            prop_assert!(!platform_support.is_supported(&unsupported));
        }

        #[test]
        fn all_platforms_supported_if_none_are_specified(identifier in platform_identifier_strategy()) {
            let platforms = vec![];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            prop_assert!(platform_support.is_supported(&identifier))
        }

        #[test]
        fn conflicting_platforms(identifier in platform_identifier_strategy()) {
            let identifier_str = identifier.to_string();
            let identifier_str_negated = format!("!{identifier}");
            let platforms = vec![identifier_str, identifier_str_negated];
            let _ = PlatformSupport::parse(&platforms).unwrap_err();
        }

        #[test]
        fn extended_platforms_supported_if_supported(identifier in platform_identifier_strategy()) {
            let identifier_str = identifier.to_string();
            let platforms = vec![identifier_str];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            for identifier in identifier.get_extended_platforms() {
                prop_assert!(platform_support.is_supported(&identifier))
            }
        }

        #[test]
        fn sub_platforms_unsupported_if_unsupported(identifier in platform_identifier_strategy()) {
            let identifier_str = format!("!{identifier}");
            let platforms = vec![identifier_str];
            let platform_support = PlatformSupport::parse(&platforms).unwrap();
            for identifier in identifier.get_subsets() {
                prop_assert!(!platform_support.is_supported(&identifier))
            }
        }

        #[test]
        fn conflicting_extended_platform_definitions(identifier in platform_identifier_strategy()) {
            let extended_platforms = identifier.get_extended_platforms();
            if extended_platforms.is_empty() {
                return Ok(());
            }
            let supported_str = identifier.to_string();
            let mut platforms: Vec<String> = extended_platforms.into_iter().map(|ident| format!("!{ident}")).collect();
            platforms.push(supported_str);
            let _ = PlatformSupport::parse(&platforms).unwrap_err();
        }
    }
}
