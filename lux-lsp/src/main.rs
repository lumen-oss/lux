use std::borrow::Cow;
use std::io::Write;
use std::sync::Mutex;

use eyre::{Context, Result};
use lux_lib::progress::client::ProgressMessage;
use lux_lib::workspace::Workspace;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tower_lsp_server::ls_types::notification;
use tower_lsp_server::ls_types::{
    InitializeParams, InitializeResult, InitializedParams, MessageType, ProgressParams,
    ProgressParamsValue, ProgressToken, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressEnd, WorkDoneProgressReport,
};
use tower_lsp_server::{jsonrpc, Client, LanguageServer, LspService, Server};

use lux_lib::progress;

use crate::tempfile::TempFile;

mod tempfile;

#[derive(Debug)]
struct Backend {
    client: Client,
    workspace: Mutex<Option<Workspace>>,
}

impl Backend {
    async fn start_socket_listener(&self, workspace: &Workspace) -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();

        let port_path = progress::lsp_port_path(workspace);

        let mut tempfile =
            TempFile::create(&port_path).wrap_err("failed to create temp file for port")?;

        tracing::info!("progress listener on 127.0.0.1:{port}");

        tempfile.write_all(port.to_string().as_bytes())?;

        let client = self.client.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                tracing::debug!("progress connection from {addr}");
                                let client = client.clone();
                                tokio::spawn(handle_connection(stream, client));
                            }
                            Err(e) => {
                                tracing::error!("accept error: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }
}

async fn handle_connection(stream: TcpStream, client: Client) {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let msg = match serde_json::from_str::<ProgressMessage>(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("invalid progress message: {e}");
                continue;
            }
        };
        relay_progress(&client, msg).await;
    }
}

async fn relay_progress(client: &Client, msg: ProgressMessage) {
    match msg {
        ProgressMessage::Begin { id, title } => {
            client
                .send_notification::<notification::Progress>(ProgressParams {
                    token: ProgressToken::Number(id),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                        WorkDoneProgressBegin {
                            title,
                            ..Default::default()
                        },
                    )),
                })
                .await;
        }
        ProgressMessage::Report { id, message } => {
            client
                .send_notification::<notification::Progress>(ProgressParams {
                    token: ProgressToken::Number(id),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                        WorkDoneProgressReport {
                            message: Some(message),
                            ..Default::default()
                        },
                    )),
                })
                .await;
        }
        ProgressMessage::End { id } => {
            client
                .send_notification::<notification::Progress>(ProgressParams {
                    token: ProgressToken::Number(id),
                    value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                        WorkDoneProgressEnd { message: None },
                    )),
                })
                .await;
        }
    }
}

impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> Result<InitializeResult, jsonrpc::Error> {
        let workspace = params
            .workspace_folders
            .and_then(|f| f.into_iter().next())
            .map(|folder| {
                folder.uri.to_file_path().map(Cow::into_owned).ok_or(
                    jsonrpc::Error::invalid_params("invalid workspace folder URI"),
                )
            })
            .transpose()?
            .map_or_else(Workspace::current, Workspace::from_exact)
            .map_err(|_| jsonrpc::Error::internal_error())?;

        match workspace {
            Some(ws) => {
                if let Ok(mut lock) = self.workspace.lock() {
                    *lock = Some(ws);
                }
            }
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        "no workspace found; progress and diagnostics disabled",
                    )
                    .await;
            }
        }

        Ok(InitializeResult::default())
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "lx-lsp initialized")
            .await;

        #[allow(clippy::unwrap_used)]
        let workspace = self.workspace.lock().unwrap().clone();

        match workspace {
            Some(ref ws) => {
                if let Err(e) = self.start_socket_listener(ws).await {
                    self.client
                        .log_message(
                            MessageType::ERROR,
                            format!("failed to start progress listener: {e}"),
                        )
                        .await;
                }
            }
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        "no workspace found; progress and diagnostics disabled",
                    )
                    .await;
            }
        }
    }

    async fn shutdown(&self) -> Result<(), jsonrpc::Error> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        workspace: Mutex::new(None),
    });

    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}
