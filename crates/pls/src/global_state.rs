use lsp_types::*;
use lsp_server::{Message, Connection};
use crossbeam_channel::{select, Receiver, Sender};

use std::path::PathBuf;

use pls_types::SegmentPool;

use crate::config::Config;
use crate::registry::{NotificationRegistry};
use crate::messages::Task;
use crate::stubs::FileMapping;

/// Inspired by `rust-analyzer`
pub struct GlobalState {
    pub config: Config,
    pub connection: lsp_server::Connection,
    pub notification_registry: NotificationRegistry,

    pub worker_send: Sender<Task>,
    pub worker_recv: Receiver<Task>,

    pub fqn_interns: SegmentPool,
    pub stub_mappings: FileMapping,
}

impl GlobalState {
    pub fn new(stubs_filename: &str, connection: Connection) -> anyhow::Result<Self> {
        let (id, value) = connection.initialize_start()?;

        // maintain backwards compatibility; we still favour `workspace_folders` over `root_uri`
        #[allow(deprecated)]
        let InitializeParams {
            root_uri,
            workspace_folders,
            ..
        } = serde_json::from_value(value).expect("unable to serialize init params");
        connection.initialize_finish(
            id,
            serde_json::json!({
                "capabilities": supported_capabilities(),
                "serverInfo": {
                    "name": env!("CARGO_PKG_NAME"),
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;

        let config = Config::new(
            workspace_folders.unwrap_or(vec![]),
            root_uri,
            PathBuf::from(stubs_filename),
        );
        let notification_registry = NotificationRegistry::default();
        let (worker_send, worker_recv) = crossbeam_channel::unbounded();
        worker_send.send(Task::AnalyzeStubs).expect("stubs should be available for analysis");

        let fqn_interns = SegmentPool::new();
        let stub_mappings = FileMapping::default();

        let x = Self {
            connection,
            config,
            notification_registry,
            fqn_interns,
            stub_mappings,

            worker_send,
            worker_recv,
        };

        Ok(x)
    }

    pub fn main_loop(&mut self) {
        loop {
            select! {
                recv(&self.connection.receiver) -> msg => {
                    match msg {
                        Ok(Message::Request(req)) => {
                            if let Ok(true) = self.connection.handle_shutdown(&req) {
                                return;
                            }
                        }
                        Ok(Message::Notification(not)) => {
                        }
                        Ok(Message::Response(resp)) => {
                            log::error!("Unexpected response: {:?}", resp);
                        }
                        Err(e) => {
                            log::error!("Err in receiving connection message: {:?}", e);
                        }
                    }
                }
                recv(&self.worker_recv) -> task => {
                    match task {
                        Ok(Task::AnalyzeStubs) => {
                            match FileMapping::from_filename(&self.config.stubs_filename) {
                                Ok(mapping) => self.stub_mappings = mapping,
                                Err(e) => log::error!("Err in reading php stubs: {:?}", e),
                            }
                        }
                        Err(e) => log::error!("Err in receiving worker tasks: {:?}", e),
                    }
                }
            }
        }
    }
}

fn supported_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_symbol_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::SOURCE]),
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: Some(false),
            },
            resolve_provider: Some(true),
        })),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    }
}
