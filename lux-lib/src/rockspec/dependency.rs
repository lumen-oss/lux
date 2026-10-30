use std::collections::HashMap;

use serde::Deserialize;

use super::external_dependency;

/// The dependency table a set of dependencies belongs to.
///
/// Unlike lua_dependency::Kind, this also covers external (C library) dependencies.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
    External(HashMap<String, external_dependency::Spec>),
}

impl<T> Kind<T> {
    pub fn as_ref(&self) -> Kind<&T> {
        match *self {
            Self::Regular(ref x) => Kind::Regular(x.iter().collect()),
            Self::Build(ref x) => Kind::Build(x.iter().collect()),
            Self::Test(ref x) => Kind::Test(x.iter().collect()),
            Self::External(ref x) => Kind::External(x.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ottavino::{Closure, Executor, Fuel, Lua, Value};
    use ottavino_util::serde::from_value;
    use path_slash::PathBufExt;

    use super::*;

    fn eval_lua<T: serde::de::DeserializeOwned>(code: &str) -> Result<T, ottavino::ExternError> {
        Lua::core().try_enter(|ctx| {
            let closure = Closure::load(ctx, None, code.as_bytes())?;
            let executor = Executor::start(ctx, closure.into(), ());
            executor.step(ctx, &mut Fuel::with(i32::MAX))?;
            from_value(executor.take_result::<Value<'_>>(ctx)??).map_err(ottavino::Error::from)
        })
    }

    #[test]
    fn dependency_kind_from_lua() {
        let regular_deps: Kind<crate::package::Req> =
            eval_lua(r#"return { regular = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let build_deps: Kind<crate::package::Req> =
            eval_lua(r#"return { build = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let test_deps: Kind<crate::package::Req> =
            eval_lua(r#"return { test = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let external_deps: Kind<external_dependency::Spec> = eval_lua(
            r#"return { external = { foo = { header = "foo.h", library = "libfoo.so" }, bar = { header = "bar.h" } } }"#,
        )
        .unwrap();

        assert!(matches!(regular_deps, Kind::Regular(ref deps) if deps.len() == 2));
        assert!(matches!(build_deps, Kind::Build(ref deps) if deps.len() == 2));
        assert!(matches!(test_deps, Kind::Test(ref deps) if deps.len() == 2));
        match external_deps {
            Kind::External(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(
                    deps["foo"].header.as_ref().unwrap().to_slash_lossy(),
                    "foo.h"
                );
                assert_eq!(
                    deps["foo"].library.as_ref().unwrap().to_slash_lossy(),
                    "libfoo.so"
                );
                assert_eq!(
                    deps["bar"].header.as_ref().unwrap().to_slash_lossy(),
                    "bar.h"
                );
                assert!(deps["bar"].library.is_none());
            }
            _ => panic!("Expected external dependencies"),
        }

        let _err: ottavino::ExternError =
            eval_lua::<Kind<external_dependency::Spec>>("return {}").unwrap_err();
    }
}
