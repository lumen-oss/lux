//! Special utilities for the Lua bridge.

use tokio::runtime::{Builder, Runtime};

use lazy_static::lazy_static;

lazy_static! {
    static ref LUA_RUNTIME: Runtime = {
        let span = tracing::debug_span!("Initialising Lua runtime");
        let _enter = span.enter();
        #[allow(clippy::expect_used)]
        Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to initialise Lua runtime")
    };
}

pub fn lua_runtime() -> &'static Runtime {
    &LUA_RUNTIME
}
