use std::convert::Infallible;

use serde::Deserialize;

use super::{PartialOverride, PerPlatform, PlatformOverridable};

/// An undocumented part of the rockspec format.
///
/// Specifies additional install options
#[derive(Clone, Debug, PartialEq, Deserialize, lux_macros::DisplayAsLuaKV)]
#[display_lua(key = "deploy")]
pub struct DeploySpec {
    /// Whether to wrap installed Lua bin scripts to be executed with
    /// the detected or configured Lua installation.
    /// Defaults to `true`.
    #[serde(default = "default_wrap_bin_scripts")]
    pub wrap_bin_scripts: bool,
}

impl Default for DeploySpec {
    fn default() -> Self {
        Self {
            wrap_bin_scripts: true,
        }
    }
}

impl PartialOverride for DeploySpec {
    type Err = Infallible;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        Ok(Self {
            wrap_bin_scripts: override_spec.wrap_bin_scripts,
        })
    }
}

impl PlatformOverridable for DeploySpec {
    type Err = Infallible;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

fn default_wrap_bin_scripts() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use crate::lua_rockspec::DisplayAsLuaKV;

    use super::*;

    fn eval_lua_global<T: serde::de::DeserializeOwned>(code: &str, key: &'static str) -> T {
        use ottavino::{Closure, Executor, Fuel, Lua};
        use ottavino_util::serde::from_value;
        Lua::core()
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, None, code.as_bytes())?;
                let executor = Executor::start(ctx, closure.into(), ());
                executor.step(ctx, &mut Fuel::with(i32::MAX))?;
                from_value(ctx.globals().get_value(ctx, key)).map_err(ottavino::Error::from)
            })
            .unwrap()
    }

    #[test]
    pub fn deploy_spec_roundtrip_true() {
        let spec = DeploySpec {
            wrap_bin_scripts: true,
        };
        let lua = spec.display_lua().to_string();
        let restored: DeploySpec = eval_lua_global(&lua, "deploy");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn deploy_spec_roundtrip_false() {
        let spec = DeploySpec {
            wrap_bin_scripts: false,
        };
        let lua = spec.display_lua().to_string();
        let restored: DeploySpec = eval_lua_global(&lua, "deploy");
        assert_eq!(spec, restored);
    }
}
