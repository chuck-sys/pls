use std::sync::Arc;
use std::collections::HashMap;
use std::panic::RefUnwindSafe;

use serde::de::DeserializeOwned;
use lsp_server::{Request, Notification};

use crate::global_state::GlobalState;

pub type AsyncCallback = Arc<
    dyn Fn(&GlobalState, serde_json::Value) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
>;

pub type SyncCallback = Box<
    dyn Fn(&mut GlobalState, serde_json::Value) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
>;

#[derive(Default)]
pub struct NotificationRegistry {
    async_handlers: HashMap<String, AsyncCallback>,
    sync_mut_handlers: HashMap<String, SyncCallback>,
}

impl NotificationRegistry {
    pub fn on<N, F>(&mut self, handler: F) -> &mut Self
        where
            N: lsp_types::notification::Notification,
            N::Params: DeserializeOwned,
            F: Fn(&GlobalState, N::Params) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
    {
        let method = N::METHOD.to_string();
        self.async_handlers.insert(method, Arc::new(move |session, params| {
            let parsed_params: N::Params = serde_json::from_value(params)?;
            handler(session, parsed_params)?;
            Ok(())
        }));
        self
    }

    pub fn on_mut<N, F>(&mut self, handler: F) -> &mut Self
        where
            N: lsp_types::notification::Notification,
            N::Params: DeserializeOwned,
            F: Fn(&mut GlobalState, N::Params) -> anyhow::Result<()> + Send + Sync + RefUnwindSafe + 'static,
    {
        let method = N::METHOD.to_string();
        self.sync_mut_handlers.insert(method, Box::new(move |session, params| {
            let parsed_params: N::Params = serde_json::from_value(params)?;
            handler(session, parsed_params)?;
            Ok(())
        }));
        self
    }

    pub fn exec(
        &self,
        state: &GlobalState,
        not: Notification,
    ) -> anyhow::Result<()> {
        let cb = Arc::clone(
            self.async_handlers
                .get(&not.method)
                .ok_or(
                    anyhow::anyhow!("Handler for method `{}` not found", &not.method)
                )?
        );

        // state.pool.spawn(
            // std::panic::AssertUnwindSafe(move || {cb(&state, not.params.clone());}));
                // match salsa::Cancelled::catch(|| cb(&state, not.params.clone())) {
                //     Err(e) => log::error!("Cancelled notification: {e}"),
                //     Ok(_) => log::debug!("Executed notification: {:?}", not),
                // }
            // }),
        // );

        Ok(())
    }

    pub fn exec_mut(
        &self,
        state: &mut GlobalState,
        not: Notification,
    ) -> anyhow::Result<()> {
        let cb = self.sync_mut_handlers.get(&not.method).ok_or(anyhow::anyhow!("Handler for method `{}` not found", &not.method))?;
        cb(state, not.params.clone())?;

        Ok(())
    }
}

#[derive(Default)]
pub struct RequestRegistry {
    async_handlers: HashMap<String, AsyncCallback>,
}

impl RequestRegistry {
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

    pub fn exec(
        &self,
        // session: &PlsSession,
        req: Request,
    ) -> anyhow::Result<()> {
        let cb = Arc::clone(
            self.async_handlers
                .get(&req.method)
                .ok_or(
                    anyhow::anyhow!("Handler for method `{}` not found", &req.method)
                )?
        );

        // let db = session.db.clone();
        // session.pool.spawn(
        //     std::panic::AssertUnwindSafe(move || {
        //         match salsa::Cancelled::catch(|| cb(&db, req.params.clone())) {
        //             Err(e) => log::error!("Cancelled notification: {e}"),
        //             Ok(_) => log::debug!("Executed notification: {:?}", req),
        //         }
        //     }),
        // );

        Ok(())
    }
}
