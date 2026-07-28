use std::io::{BufWriter, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, RwLock};
use std::{io, num};

use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::workspace::Workspace;

pub(crate) static CLIENT: RwLock<Option<Arc<LspClient>>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressMessage {
    Begin { id: i32, title: String },
    Report { id: i32, message: String },
    End { id: i32 },
}

pub(crate) struct LspClient {
    writer: Mutex<BufWriter<TcpStream>>,
}

impl LspClient {
    pub(crate) fn connect(workspace: &Workspace) -> Result<Self, ConnectError> {
        let port_path = super::lsp_port_path(workspace);

        let port: u16 = std::fs::read_to_string(&port_path)
            .map_err(ConnectError::ReadPort)?
            .trim()
            .parse()
            .map_err(ConnectError::ParsePort)?;
        let stream = TcpStream::connect(("127.0.0.1", port))
            .map_err(|source| ConnectError::Connect { port, source })?;

        Ok(Self {
            writer: Mutex::new(BufWriter::new(stream)),
        })
    }

    pub(crate) fn send(&self, msg: &ProgressMessage) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = serde_json::to_writer(&mut *w, msg);
            let _ = writeln!(w);
            let _ = w.flush();
        }
    }
}

#[derive(Debug, Diagnostic, Error)]
pub(crate) enum ConnectError {
    #[error("failed to read LSP port file: {0}")]
    ReadPort(#[from] io::Error),
    #[error("invalid port in LSP port file: {0}")]
    ParsePort(#[from] num::ParseIntError),
    #[error("failed to connect to lx-lsp at 127.0.0.1:{port}: {source}")]
    Connect {
        port: u16,
        #[source]
        source: io::Error,
    },
}
