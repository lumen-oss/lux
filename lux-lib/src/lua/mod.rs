//! Special utilities for the Lua bridge.

pub mod binary;
pub mod detect;
pub mod installation;
pub mod version;

pub use binary::Binary;
pub use installation::Installation;
pub use version::Version;

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

pub fn runtime() -> &'static Runtime {
    &LUA_RUNTIME
}
