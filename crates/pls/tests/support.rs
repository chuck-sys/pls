use serde::Serialize;

use lsp_server::{Notification, Request, RequestId, Connection};
use lsp_types::*;

use std::str::FromStr;

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

    pub fn request<R>(&mut self, params: R::Params)
        where R: lsp_types::request::Request,
              R::Params: Serialize,
    {
        self.conn.sender.send(
            Request::new(
                RequestId::from(self.next_req_id as i32),
                R::METHOD.to_owned(),
                params,
            ).into()
        ).unwrap();

        self.next_req_id += 1;
    }

    pub fn notify<N>(&self, params: N::Params)
        where N: lsp_types::notification::Notification,
              N::Params: Serialize,
    {
        self.conn.sender.send(
            Notification::new(
                N::METHOD.to_owned(),
                params,
            ).into()
        ).unwrap();
    }

    pub fn initialize(&mut self) {
        self.request::<request::Initialize>(InitializeParams {
            process_id: None,
            workspace_folders: Some(vec![
                WorkspaceFolder {
                    uri: Uri::from_str("file://.").unwrap(),
                    name: String::from("folder"),
                },
            ]),
            ..Default::default()
        });
        self.notify::<notification::Initialized>(InitializedParams {});
    }

    pub fn shutdown(&mut self) {
        self.request::<request::Shutdown>(());
        self.notify::<notification::Exit>(());
    }
}
