use std::sync::Arc;
use std::collections::HashMap;
use std::panic::RefUnwindSafe;

use lsp_types::notification::DidOpenTextDocument;
use serde::de::DeserializeOwned;
use lsp_server::{Request, Notification};

use crate::{global_state::GlobalState, handlers};

pub type SyncCallback = Box<
    dyn Fn(&mut GlobalState, serde_json::Value) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
>;

pub struct NotificationRegistry {
    handlers: HashMap<String, SyncCallback>,
}

impl NotificationRegistry {
    pub fn on<N, F>(&mut self, handler: F) -> &mut Self
        where
            N: lsp_types::notification::Notification,
            N::Params: DeserializeOwned,
            F: Fn(&mut GlobalState, N::Params) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
    {
        let method = N::METHOD.to_string();
        self.handlers.insert(method, Box::new(move |session, params| {
            let parsed_params: N::Params = serde_json::from_value(params)?;
            handler(session, parsed_params)?;
            Ok(())
        }));
        self
    }

    pub fn exec(
        &self,
        state: &mut GlobalState,
        not: Notification,
    ) -> anyhow::Result<()> {
        let cb = self.handlers.get(&not.method).ok_or(anyhow::anyhow!("Handler for method `{}` not found", &not.method))?;
        cb(state, not.params.clone())?;

        Ok(())
    }
}

impl Default for NotificationRegistry {
    fn default() -> Self {
        let mut me = Self { handlers: Default::default() };
        me.on::<DidOpenTextDocument, _>(handlers::notification::did_open_text_document);

        me
    }
}

// #[derive(Default)]
// pub struct RequestRegistry {
//     async_handlers: HashMap<String, AsyncCallback>,
// }

// impl RequestRegistry {
    // pub fn on<N, F>(&mut self, handler: F) -> &mut Self
    //     where
    //         N: lsp_types::request::Request,
    //         N::Params: DeserializeOwned,
    //         F: Fn(&Db, N::Params) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
    // {
    //     let method = N::METHOD.to_string();
    //     self.async_handlers.insert(method, Arc::new(move |session, params| {
    //         let parsed_params: N::Params = serde_json::from_value(params)?;
    //         handler(session, parsed_params)?;
    //         Ok(())
    //     }));
    //     self
    // }

    // pub fn exec(
    //     &self,
    //     // session: &PlsSession,
    //     req: Request,
    // ) -> anyhow::Result<()> {
    //     let cb = Arc::clone(
    //         self.async_handlers
    //             .get(&req.method)
    //             .ok_or(
    //                 anyhow::anyhow!("Handler for method `{}` not found", &req.method)
    //             )?
    //     );

        // let db = session.db.clone();
        // session.pool.spawn(
        //     std::panic::AssertUnwindSafe(move || {
        //         match salsa::Cancelled::catch(|| cb(&db, req.params.clone())) {
        //             Err(e) => log::error!("Cancelled notification: {e}"),
        //             Ok(_) => log::debug!("Executed notification: {:?}", req),
        //         }
        //     }),
        // );

        // Ok(())
    // }
// }
