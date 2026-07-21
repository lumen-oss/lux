use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::{fmt, io, num};

use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::workspace::Workspace;

static CLIENT: RwLock<Option<Arc<LspClient>>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressMessage {
    Begin { id: i32, title: String },
    Report { id: i32, message: String },
    End { id: i32 },
}

pub fn progress_layer() -> LspProgressLayer {
    LspProgressLayer::new()
}

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

struct LspClient {
    writer: Mutex<BufWriter<TcpStream>>,
}

impl LspClient {
    fn connect(workspace: &Workspace) -> Result<Self, ConnectError> {
        let port_path = lsp_port_path(workspace);

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

    fn send(&self, msg: &ProgressMessage) {
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

pub struct LspProgressLayer {
    span_ids: Mutex<HashMap<Id, i32>>,
    next_id: AtomicI32,
}

impl LspProgressLayer {
    fn new() -> Self {
        Self {
            span_ids: Mutex::new(HashMap::new()),
            next_id: AtomicI32::new(1),
        }
    }

    fn next_id(&self) -> i32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl<S> Layer<S> for LspProgressLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        with_client(|client| {
            let pid = self.next_id();
            if let Ok(mut ids) = self.span_ids.lock() {
                ids.insert(id.clone(), pid);
            }
            client.send(&ProgressMessage::Begin {
                id: pid,
                title: attrs.metadata().name().to_string(),
            });
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if *event.metadata().level() > tracing::Level::INFO {
            return;
        }

        let pid = ctx
            .event_span(event)
            .and_then(|span_ref| self.span_ids.lock().ok()?.get(&span_ref.id()).copied());

        if let Some(pid) = pid {
            let mut visitor = MessageVisitor { message: None };
            event.record(&mut visitor);

            if let Some(message) = visitor.message {
                with_client(|client| {
                    client.send(&ProgressMessage::Report { id: pid, message });
                });
            }
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Some(pid) = self
            .span_ids
            .lock()
            .ok()
            .and_then(|mut ids| ids.remove(&id))
        {
            with_client(|client| {
                client.send(&ProgressMessage::End { id: pid });
            });
        }
    }
}

struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

fn with_client<F>(f: F)
where
    F: FnOnce(&LspClient),
{
    let client = CLIENT
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().map(Arc::clone));
    if let Some(ref c) = client {
        f(c);
    }
}
