use lsp_server::{ExtractError, RequestId};

use crate::global_state::GlobalState;

/// NotificationDispatcher from rust-analyzer project.
///
/// Writing the different types and extractions is tedious and annoying, so we reuse some effort.
///
/// See https://github.com/rust-lang/rust-analyzer/blob/a071c5cb042c85b99133ce1bafabe0510699b0ac/crates/rust-analyzer/src/handlers/dispatch.rs#L393
pub struct NotificationDispatcher<'a> {
    pub state: &'a mut GlobalState,

    /// Must be an [`Option`] because when we [`lsp_server::Notification::extract()`] the guts, it
    /// takes ownership, and can be empty.
    pub notification: Option<lsp_server::Notification>,
}

impl NotificationDispatcher<'_> {
    pub fn handle<N>(&mut self, f: fn(&mut GlobalState, N::Params) -> Result<(), std::convert::Infallible>) -> &mut Self
    where
        N: lsp_types::notification::Notification,
        N::Params: serde::de::DeserializeOwned + Send + std::fmt::Debug,
    {
        let notification = match self.notification.take() {
            Some(n) => n,
            None => return self,
        };

        let params = match notification.extract::<N::Params>(N::METHOD) {
            Ok(it) => it,
            Err(ExtractError::MethodMismatch(notification)) => {
                self.notification = Some(notification);
                return self;
            }
            Err(ExtractError::JsonError { method, error }) => {
                // We don't need to replace the notification because it isn't valid json
                log::error!("issue with extracting notification `{method}` json: {error}");
                return self;
            }
        };

        if let Err(e) = f(self.state, params) {
            log::error!("notification handler `{}` failed: {:?}", N::METHOD, e);
        }

        self
    }

    pub fn finish(&mut self) {
        if let Some(notification) = self.notification.take() {
            log::warn!("unhandled notification: {:?}", notification);
        }
    }
}

pub struct RequestDispatcher<'a> {
    pub state: &'a mut GlobalState,

    pub request: Option<lsp_server::Request>,
}

impl RequestDispatcher<'_> {
    pub fn handle<R>(&mut self, f: fn(&mut GlobalState, (RequestId, R::Params)) -> Result<(), std::convert::Infallible>) -> &mut Self
    where
        R: lsp_types::request::Request,
        R::Params: serde::de::DeserializeOwned + Send + std::fmt::Debug,
    {
        let request = match self.request.take() {
            Some(n) => n,
            None => return self,
        };

        let (request_id, params) = match request.extract::<R::Params>(R::METHOD) {
            Ok(it) => it,
            Err(ExtractError::MethodMismatch(request)) => {
                self.request = Some(request);
                return self;
            }
            Err(ExtractError::JsonError { method, error }) => {
                // We don't need to replace the request because it isn't valid json
                log::error!("issue with extracting request `{method}` json: {error}");
                return self;
            }
        };

        if let Err(e) = f(self.state, (request_id, params)) {
            log::error!("request handler `{}` failed: {:?}", R::METHOD, e);
        }

        self
    }

    pub fn finish(&mut self) {
        if let Some(request) = self.request.take() {
            log::warn!("unhandled request: {:?}", request);
        }
    }
}
