mod dispatch;
mod notification;
mod request;

use lsp_server::{Request, Response};

use lsp_types::request::*;
use lsp_types::notification::*;

use crate::global_state::GlobalState;
use notification::*;
use request::*;

pub fn handle_notification(
    state: &mut GlobalState,
    notification: lsp_server::Notification,
) -> Result<(), std::convert::Infallible> {
    dispatch::NotificationDispatcher {
        state,
        notification: Some(notification),
    }
    .handle::<DidOpenTextDocument>(did_open_text_document)
    .finish();

    Ok(())
}

pub fn handle_request(state: &mut GlobalState, request: Request) {
    dispatch::RequestDispatcher {
        state,
        request: Some(request),
    }
    .handle::<CodeActionRequest>(code_action)
    .finish();
}

pub fn handle_response(_: &mut GlobalState, response: Response) {
    log::warn!("received a response message: {:?}", response);
}
