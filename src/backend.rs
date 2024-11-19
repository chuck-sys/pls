use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use async_channel::{Receiver, Sender};

use std::path::PathBuf;

use crate::msg::{MsgFromServer, MsgToServer};

pub struct Backend {
    client: Client,
    receiver_from_server: Receiver<MsgFromServer>,
    sender_to_server: Sender<MsgToServer>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let (sender_to_backend, receiver_from_server) = async_channel::unbounded();
        let (sender_to_server, receiver_from_backend) = async_channel::unbounded();
        let mut server =
            crate::server::Server::new(client.clone(), sender_to_backend, receiver_from_backend);
        tokio::spawn(async move {
            server.serve().await;
        });

        Self {
            client,
            receiver_from_server,
            sender_to_server,
        }
    }

    pub async fn send(&self, msg: MsgToServer) {
        if let Err(x) = self.sender_to_server.send(msg.clone()).await {
            self.client.log_message(MessageType::ERROR, msg).await;
            self.client.log_message(MessageType::ERROR, x).await;
        }
    }

    pub async fn recv(&self) -> Option<MsgFromServer> {
        match self.receiver_from_server.recv().await {
            Ok(msg) => Some(msg),
            Err(x) => {
                self.client.log_message(MessageType::ERROR, x).await;
                None
            }
        }
    }
}

/**
 * Composer files paths should always exist.
 *
 * Please remember to check existence because there is a chance that it gets deleted.
 */
fn get_composer_files(workspace_folders: &Vec<WorkspaceFolder>) -> Result<Vec<PathBuf>> {
    let mut composer_files = vec![];
    for folder in workspace_folders {
        if let Ok(path) = folder.uri.to_file_path() {
            let composer_file = path.join("composer.json");
            if !composer_file.exists() {
                continue;
            }

            composer_files.push(composer_file);
        } else {
            continue;
        }
    }

    Ok(composer_files)
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut workspace_folders = params.workspace_folders.unwrap_or(vec![]);
        if workspace_folders.len() == 0 {
            if let Some(root_uri) = params.root_uri {
                workspace_folders.push(WorkspaceFolder {
                    uri: root_uri.clone(),
                    name: root_uri.to_string(),
                });
            }
        }

        if workspace_folders.len() == 0 {
            self.client
                .log_message(
                    MessageType::LOG,
                    "unable to find workspace folders, root paths, or root uris",
                )
                .await;
        } else {
            self.client
                .log_message(
                    MessageType::LOG,
                    format!(
                        "found {} workspace folders: {:?}",
                        workspace_folders.len(),
                        &workspace_folders
                    ),
                )
                .await;
        }

        // TODO check workspace folders for `composer.json` and read namespaces with PSR-4 and
        // PSR-0 (maybe support it??)
        let composer_files = get_composer_files(&workspace_folders)?;
        self.send(MsgToServer::ComposerFiles(composer_files)).await;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.send(MsgToServer::Shutdown).await;
        self.client
            .log_message(MessageType::LOG, "server thread has shutdown")
            .await;
        Ok(())
    }

    async fn did_open(&self, data: DidOpenTextDocumentParams) {
        self.send(MsgToServer::DidOpen {
            url: data.text_document.uri,
            text: data.text_document.text,
            version: data.text_document.version,
        })
        .await;
    }

    async fn did_change(&self, data: DidChangeTextDocumentParams) {
        self.send(MsgToServer::DidChange {
            url: data.text_document.uri,
            version: data.text_document.version,
            content_changes: data.content_changes,
        })
        .await;
    }

    async fn document_symbol(
        &self,
        data: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        self.send(MsgToServer::DocumentSymbol(data.text_document.uri))
            .await;

        match self.recv().await {
            Some(MsgFromServer::NestedSymbols(symbols)) => {
                Ok(Some(DocumentSymbolResponse::Nested(symbols)))
            },
            _ => Ok(None),
        }
    }
}
