//! Special utilities for the Lua bridge.

use tokio::runtime::{Builder, Runtime};

use lazy_static::lazy_static;

lazy_static! {
    static ref LUA_RUNTIME: Runtime = {
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
