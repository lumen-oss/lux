use crate::{
    progress::client::{LspClient, CLIENT},
    workspace::Workspace,
};
use std::{path::PathBuf, sync::Arc};

pub mod client;
pub mod layer;

pub fn lsp_port_path(workspace: &Workspace) -> PathBuf {
    workspace.root().join(".lux").join("lsp-port")
}

pub fn set_connection(workspace: &Workspace) {
    match LspClient::connect(workspace) {
        Ok(client) => {
            if let Ok(mut guard) = CLIENT.write() {
                *guard = Some(Arc::new(client));
            }
        }
        Err(_) => {
            tracing::trace!("no lx-lsp server found; LSP progress disabled");
        }
    }
}
