use std::collections::HashMap;
use std::panic::RefUnwindSafe;

use lsp_server::{Notification, Request, RequestId};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
};
use lsp_types::request::CodeActionRequest;
use serde::de::DeserializeOwned;

use crate::{global_state::GlobalState, handlers};

pub type NotificationCallback = Box<
    dyn Fn(&mut GlobalState, serde_json::Value) -> anyhow::Result<()>
        + Send
        + Sync
        + RefUnwindSafe
        + 'static,
>;

pub type RequestCallback = Box<
    dyn Fn(RequestId, &mut GlobalState, serde_json::Value) -> anyhow::Result<()>
        + Send
        + Sync
        + RefUnwindSafe
        + 'static,
>;

pub struct NotificationRegistry {
    handlers: HashMap<String, NotificationCallback>,
}

impl NotificationRegistry {
    pub fn on<N, F>(&mut self, handler: F) -> &mut Self
    where
        N: lsp_types::notification::Notification,
        N::Params: DeserializeOwned,
        F: Fn(&mut GlobalState, N::Params) -> anyhow::Result<()>
            + Send
            + Sync
            + RefUnwindSafe
            + 'static,
    {
        let method = N::METHOD.to_string();
        self.handlers.insert(
            method,
            Box::new(move |session, params| {
                let parsed_params: N::Params = serde_json::from_value(params)?;
                handler(session, parsed_params)?;
                Ok(())
            }),
        );
        self
    }

    pub fn exec(&self, state: &mut GlobalState, not: Notification) -> anyhow::Result<()> {
        let cb = self.handlers.get(&not.method).ok_or(anyhow::anyhow!(
            "Handler for method `{}` not found",
            &not.method
        ))?;
        cb(state, not.params.clone())?;

        Ok(())
    }
}

impl Default for NotificationRegistry {
    fn default() -> Self {
        let mut me = Self {
            handlers: Default::default(),
        };
        me.on::<DidOpenTextDocument, _>(handlers::notification::did_open_text_document)
            .on::<DidChangeTextDocument, _>(handlers::notification::did_change_text_document)
            .on::<DidSaveTextDocument, _>(handlers::notification::did_save_text_document)
            .on::<DidCloseTextDocument, _>(handlers::notification::did_close_text_document);

        me
    }
}

pub struct RequestRegistry {
    handlers: HashMap<String, RequestCallback>,
}

impl Default for RequestRegistry {
    fn default() -> Self {
        let mut me = Self {
            handlers: Default::default(),
        };
        me.on::<CodeActionRequest, _>(handlers::request::code_action);

        me
    }
}

impl RequestRegistry {
    pub fn on<N, F>(&mut self, handler: F) -> &mut Self
    where
        N: lsp_types::request::Request,
        N::Params: DeserializeOwned,
        F: Fn(RequestId, &mut GlobalState, N::Params) -> anyhow::Result<()>
            + Send
            + Sync
            + RefUnwindSafe
            + 'static,
    {
        let method = N::METHOD.to_string();
        self.handlers.insert(
            method,
            Box::new(move |request_id, session, params| {
                let parsed_params: N::Params = serde_json::from_value(params)?;
                handler(request_id, session, parsed_params)?;
                Ok(())
            }),
        );
        self
    }

    pub fn exec(&self, state: &mut GlobalState, req: Request) -> anyhow::Result<()> {
        let cb = self.handlers.get(&req.method).ok_or(anyhow::anyhow!(
            "Handler for method `{}` not found",
            &req.method
        ))?;

        cb(req.id, state, req.params.clone())?;

        Ok(())
    }
}
