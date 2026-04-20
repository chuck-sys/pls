use lsp_types::*;
use lsp_server::{Message, Connection};

use std::panic::RefUnwindSafe;
use std::path::PathBuf;
use std::num::NonZeroUsize;

use crate::config::Config;
use crate::pool::Pool;
use crate::registry::{RequestRegistry, NotificationRegistry};

#[salsa::interned]
pub struct Fqn<'db> {
    #[returns(ref)]
    pub text: String,
}

#[salsa::interned]
pub struct FilePath<'db> {
    #[returns(ref)]
    pub path: PathBuf,
}

#[salsa::db]
#[derive(Clone, Default)]
pub struct PlsDatabase {
    pub storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for PlsDatabase {}

pub struct PlsSession<Db: salsa::Database> {
    pub db: Db,

    pub config: Config,
    pub connection: Connection,

    pub pool: Pool,

    notification_registry: NotificationRegistry<Db>,
}

impl<Db: salsa::Database + RefUnwindSafe> PlsSession<Db> {
    pub fn new(stubs_filename: &str, connection: Connection, db: Db) -> anyhow::Result<Self> {
        let max_threads = std::thread::available_parallelism()
            .unwrap_or_else(|_| NonZeroUsize::new(1).unwrap())
            .get();
        let pool = Pool::new(max_threads);

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
            std::path::PathBuf::from(stubs_filename),
        );

        Ok(Self {
            db,
            connection,
            config,
            pool,
        })
    }

    pub fn main_loop(&mut self) -> anyhow::Result<()> {
        for msg in &self.connection.receiver {
            match msg {
                Message::Request(req) => {
                    if self.connection.handle_shutdown(&req)? {
                        break;
                    }
                }
                Message::Notification(_not) => {}
                Message::Response(resp) => {
                    log::error!("Unexpected response: {:?}", resp)
                }
            }
        }

        Ok(())
    }
}

#[salsa::input(debug)]
pub struct SourceProgram {
    #[returns(ref)]
    pub uri: Uri,

    #[returns(ref)]
    pub text: String,

    pub version: Option<i32>,
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
