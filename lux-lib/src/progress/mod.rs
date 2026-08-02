use crate::{
    config::Config,
    progress::client::{LspClient, CLIENT},
    workspace::Workspace,
};
use std::{path::PathBuf, sync::Arc};

pub mod client;
pub mod layer;

const LUX_LSP_PORT_FILE: &str = "LUX_LSP_PORT_FILE";

pub fn lsp_port_path(workspace: &Workspace) -> PathBuf {
    match std::env::var(LUX_LSP_PORT_FILE) {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            tracing::debug!("{LUX_LSP_PORT_FILE} not set.");
            match Config::project_dirs()
                .ok()
                .and_then(|p| p.runtime_dir().map(|p| p.to_path_buf()))
            {
                // On Linux & BSD, this is /run/user/<uid>/lux/lsp-port
                Some(runtime_dir) => runtime_dir.join("lsp-port"),
                None => {
                    tracing::debug!("No runtime directory. Falling back to workspace root");
                    workspace.root().join(".lux").join("lsp-port")
                }
            }
        }
    }
}

pub fn set_connection(workspace: &Workspace) {
    match LspClient::connect(workspace) {
        Ok(client) => {
            if let Ok(mut guard) = CLIENT.write() {
                *guard = Some(Arc::new(client));
            }
        }
        Err(err) => {
            tracing::debug!(
                "no lx-lsp server found, LSP progress forwarding remains disabled: {err}"
            );
        }
    }
}
