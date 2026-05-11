use serde::Serialize;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::*;

use pls::global_state::GlobalState;
use pls::registry::{NotificationRegistry, RequestRegistry};

use std::str::FromStr;
use std::thread;
use std::time::Duration;

pub struct FakeClient {
    conn: Connection,
    next_req_id: usize,
}

impl FakeClient {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn,
            next_req_id: 1,
        }
    }

    pub fn next_response(&mut self, id: usize, limit: usize) -> anyhow::Result<Response> {
        let id = RequestId::from(id as i32);

        let mut trials = 0;

        for msg in &self.conn.receiver {
            match msg {
                Message::Response(resp) if resp.id == id => return Ok(resp.clone()),
                _ => trials += 1,
            }

            if trials >= limit {
                break;
            }
        }

        Err(anyhow::anyhow!(
            "no responses within the previous {limit} messages"
        ))
    }

    pub fn request<R>(&mut self, params: R::Params) -> usize
    where
        R: lsp_types::request::Request,
        R::Params: Serialize,
    {
        self.conn
            .sender
            .send(
                Request::new(
                    RequestId::from(self.next_req_id as i32),
                    R::METHOD.to_owned(),
                    params,
                )
                .into(),
            )
            .unwrap();

        self.next_req_id += 1;

        self.next_req_id - 1
    }

    pub fn notify<N>(&self, params: N::Params)
    where
        N: lsp_types::notification::Notification,
        N::Params: Serialize,
    {
        self.conn
            .sender
            .send(Notification::new(N::METHOD.to_owned(), params).into())
            .unwrap();
    }

    pub fn initialize(&mut self) {
        self.request::<request::Initialize>(InitializeParams {
            process_id: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Uri::from_str("file://.").unwrap(),
                name: String::from("folder"),
            }]),
            ..Default::default()
        });
        self.notify::<notification::Initialized>(InitializedParams {});
    }

    pub fn shutdown(&mut self) {
        self.request::<request::Shutdown>(());
        self.notify::<notification::Exit>(());
    }
}

#[derive(Debug)]
enum QuittingState {
    GracefulShutdown,
    ThreadTimeout(Duration),
}

pub struct TestConfig {
    pub stubs_filename: &'static str,
    pub max_test_duration: Duration,
}

pub fn run_with<F>(test_cfg: TestConfig, cb: F)
where
    F: FnOnce(&mut FakeClient),
{
    let (connection, client) = Connection::memory();
    let mut client = FakeClient::new(client);
    let (tx, rx) = crossbeam_channel::bounded(2);
    let tx2 = tx.clone();
    thread::spawn(move || {
        let mut state = GlobalState::new(test_cfg.stubs_filename, connection)
            .expect("global state initialization");
        let notification_registry = NotificationRegistry::default();
        let request_registry = RequestRegistry::default();
        state.main_loop((&notification_registry, &request_registry));

        let _ = tx2.send(QuittingState::GracefulShutdown);
    });

    client.initialize();

    cb(&mut client);

    client.shutdown();

    thread::spawn(move || {
        thread::sleep(test_cfg.max_test_duration);

        let _ = tx.send(QuittingState::ThreadTimeout(test_cfg.max_test_duration));
    });

    match rx.recv() {
        Ok(QuittingState::GracefulShutdown) => {}
        Ok(QuittingState::ThreadTimeout(t)) => {
            panic!("Timeout {t:?} passed and main loop still hasn't stopped!");
        }
        Err(e) => {
            panic!(
                "Error occurred trying to receive from shutdown channel: {:?}",
                e
            );
        }
    }
}
