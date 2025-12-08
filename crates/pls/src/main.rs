use std::env;
use std::error::Error;

use crossbeam_channel::{RecvError, select};

use lsp_server::{ProtocolError, Message, Connection};

use lsp_types::*;

mod compat;
mod config;
mod diagnostics;
mod global_state;
mod file;
mod messages;
mod scope;
mod stubs;

use config::Config;
use global_state::GlobalState;
use messages::AnalysisThreadMessage;

const VERSION_ARG: &'static str = "--version";

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    colog::init();

    // no need to include `clap` when this will suffice
    let mut stubs_filename = None;
    for (i, arg) in env::args().enumerate() {
        if i == 0 {
            continue;
        }

        if &arg == VERSION_ARG {
            log::info!(
                "{} version {}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        } else {
            stubs_filename = Some(arg);
            break;
        }
    }

    match stubs_filename {
        None => {
            log::error!(
                "missing argument: location of stubs file; e.g.: `{} phpstorm-stubs/PhpStormStubsMap.php`",
                env!("CARGO_PKG_NAME")
            );
            return Ok(());
        }
        Some(stubs_filename) => {
            log::debug!("starting server version {}", env!("CARGO_PKG_VERSION"));

            let (connection, io_threads) = Connection::stdio();
            let (id, params) = connection.initialize_start()?;

            let InitializeParams {
                root_uri,
                capabilities,
                workspace_folders,
                initialization_options,
                client_info,
                ..
            } = serde_json::from_value(params).expect("unable to serialize init params");

            // if let Some(v) = initialization_options {
            //     match serde_json::from_value(v) {
            //         Ok(v) => {}
            //         Err(e) => {
            //             log::warn!("bad init options; using defaults");
            //         }
            //     }
            // }

            connection.initialize_finish(id, serde_json::json!({
                "capabilities": supported_capabilities(),
                "serverInfo": {
                    "name": env!("CARGO_PKG_NAME"),
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }))?;

            let cfg = Config::new(workspace_folders.unwrap_or(vec![]), root_uri, std::path::PathBuf::from(stubs_filename));
            let (main_send, thread_recv) = crossbeam_channel::unbounded();
            let (thread_send, main_recv) = crossbeam_channel::unbounded();
            let state = GlobalState {
                cfg,
                connection,

                analysis_send: main_send,
                analysis_recv: main_recv,
            };

            let analysis_handle = std::thread::spawn(|| {
                for msg in thread_recv {
                    match msg {
                        AnalysisThreadMessage::Shutdown => break,
                        AnalysisThreadMessage::AnalyzeUri(uri) => todo!(),
                    }
                }
            });

            main_loop(state)?;
            io_threads.join()?;
            if let Err(e) = analysis_handle.join() {
                log::error!("could not join analysis thread after shutdown issued: {:?}", e);
            }
        }
    }

    Ok(())
}

fn main_loop(state: GlobalState) -> Result<(), RecvError> {
    loop {
        select! {
            recv(state.connection.receiver) -> msg => {
                match msg? {
                    Message::Request(request) => {
                        if let Ok(true) = state.connection.handle_shutdown(&request) {
                            let _ = state.analysis_send.send(AnalysisThreadMessage::Shutdown);
                            break;
                        }
                    },
                    Message::Response(response) => todo!(),
                    Message::Notification(notification) => todo!(),
                }
            },
            recv(state.analysis_recv) -> _ => {

            },
        }
    }

    Ok(())
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
