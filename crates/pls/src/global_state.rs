use crossbeam_channel::{Receiver, Sender, select};
use lsp_server::{Connection, Message, Notification, Request};
use lsp_types::*;

use std::collections::HashMap;
use std::path::PathBuf;

use pls_types::SegmentPool;

use crate::config::Config;
use crate::messages::Task;
use crate::registry::{NotificationRegistry, RequestRegistry};
use crate::stubs::FileMapping;

#[derive(Debug)]
pub struct FileInfo {
    pub file_name: PathBuf,
    pub content: String,
    pub php_ast: tree_sitter::Tree,
    pub phpdoc_ast: tree_sitter::Tree,
    pub version: i32,
    // pub symbols: HashMap<tree_sitter::Range, ()>,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Parsers {
    pub php: tree_sitter::Parser,
    pub phpdoc: tree_sitter::Parser,
}

impl Parsers {
    pub fn new() -> Self {
        let mut php = tree_sitter::Parser::new();
        php.set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .expect("unable to set php language parser");
        let mut phpdoc = tree_sitter::Parser::new();
        phpdoc
            .set_language(&tree_sitter_phpdoc::language())
            .expect("unable to set phpdoc language parser");

        Self { php, phpdoc }
    }

    /// TODO parse phpdoc into ast, probably requires changes to [`FileInfo`]
    pub fn parse(
        &mut self,
        content: &str,
        original_tree: Option<&tree_sitter::Tree>,
    ) -> Option<tree_sitter::Tree> {
        self.php.parse(content, original_tree)
    }
}

/// Inspired by `rust-analyzer`
pub struct GlobalState {
    pub config: Config,
    pub connection: lsp_server::Connection,

    pub worker_send: Sender<Task>,
    pub worker_recv: Receiver<Task>,

    pub fqn_interns: SegmentPool,
    pub stub_mappings: FileMapping,

    pub file_infos: HashMap<PathBuf, FileInfo>,
    pub parsers: Parsers,
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
        let (worker_send, worker_recv) = crossbeam_channel::unbounded();
        worker_send
            .send(Task::AnalyzeStubs)
            .expect("stubs should be available for analysis");

        let fqn_interns = SegmentPool::new();
        let stub_mappings = FileMapping::default();

        let x = Self {
            connection,
            config,
            fqn_interns,
            stub_mappings,

            worker_send,
            worker_recv,

            file_infos: HashMap::new(),
            parsers: Parsers::new(),
        };

        Ok(x)
    }

    pub fn main_loop(&mut self, (notif_reg, req_reg): (&NotificationRegistry, &RequestRegistry)) {
        loop {
            select! {
                recv(&self.connection.receiver) -> msg => {
                    match msg {
                        Ok(Message::Request(req)) => {
                            if let Ok(true) = self.connection.handle_shutdown(&req) {
                                return;
                            }

                            self.handle_request(req_reg, req);
                        }
                        Ok(Message::Notification(not)) => {
                            self.handle_notification(notif_reg, not)
                        }
                        Ok(Message::Response(resp)) => log::error!("Unexpected response: {resp:?}"),
                        Err(e) => {
                            log::error!("Err in receiving connection message: {e:?}");
                            break;
                        }
                    }
                }
                recv(&self.worker_recv) -> task => {
                    match task {
                        Ok(Task::AnalyzeStubs) => {
                            match FileMapping::from_filename(&self.config.stubs_filename) {
                                Ok(mapping) => self.stub_mappings = mapping,
                                Err(e) => log::error!("Err in reading php stubs: {e:?}"),
                            }
                        }
                        Ok(Task::AnalyzeFile(path)) => {
                        }
                        Err(e) => log::error!("Err in receiving worker tasks: {e:?}"),
                    }
                }
            }
        }
    }

    fn handle_request(&mut self, reg: &RequestRegistry, req: Request) {
        if let Err(e) = reg.exec(self, req) {
            log::error!("Err in handling executing request: {e:?}");
        }
    }

    fn handle_notification(&mut self, reg: &NotificationRegistry, notif: Notification) {
        if let Err(e) = reg.exec(self, notif) {
            log::error!("Err in handling executing notification: {e:?}");
        }
    }
}

fn supported_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                will_save: None,
                will_save_wait_until: None,
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(true),
                })),
            },
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
