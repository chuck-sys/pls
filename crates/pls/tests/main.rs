//! Integration tests go here
use lsp_server::{Message, RequestId, Connection, Request, Notification};
use lsp_types::request::Request as _;
use lsp_types::notification::Notification as _;
use lsp_types::*;
use serde_json::{json, Value};

use pls::global_state::GlobalState;
use std::str::FromStr;
use std::thread;
use std::time::Duration;

mod support;

const STUBS_FILENAME: &'static str = "";

#[derive(Debug)]
enum QuittingState {
    GracefulShutdown,
    ThreadTimeout(Duration),
}

#[test]
fn minimal_config_that_quits() {
    let (connection, client) = Connection::memory();
    let (tx, rx) = crossbeam_channel::bounded(2);
    let tx2 = tx.clone();
    thread::spawn(move || {
        let mut state = GlobalState::new("non-existant.txt", connection).expect("global state initialization");
        state.main_loop();

        let _ = tx2.send(QuittingState::GracefulShutdown);
    });

    client.sender.send(Request::new(RequestId::from(1), "initialize".to_string(), InitializeParams {
        process_id: None,
        workspace_folders: Some(vec![
            WorkspaceFolder {
                uri: Uri::from_str("file://.").unwrap(),
                name: String::from("folder"),
            },
        ]),
        ..Default::default()
    }).into()).unwrap();
    client.sender.send(Notification::new("initialized".to_string(), InitializedParams {}).into()).unwrap();
    client.sender.send(Request::new(RequestId::from(2), request::Shutdown::METHOD.to_owned(), Value::Null).into()).unwrap();
    client.sender.send(Notification::new("exit".to_string(), Value::Null).into()).unwrap();

    let wait_time = Duration::from_secs(2);
    thread::spawn(move || {
        thread::sleep(wait_time);

        let _ = tx.send(QuittingState::ThreadTimeout(wait_time));
    });

    match rx.recv() {
        Ok(QuittingState::GracefulShutdown) => {}
        Ok(QuittingState::ThreadTimeout(t)) => {
            panic!("Timeout {t:?} passed and main loop still hasn't stopped!");
        }
        Err(e) => {
            panic!("Error occurred trying to receive from shutdown channel: {:?}", e);
        }
    }
}
