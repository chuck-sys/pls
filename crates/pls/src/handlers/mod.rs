pub mod notification;
pub mod request;

use lsp_server::{Request, Response};

use lsp_types::request::*;
use lsp_types::notification::*;

use crate::global_state::GlobalState;
use notification::*;
use request::*;

pub fn handle_response(_: &mut GlobalState, response: Response) {
    log::warn!("received a response message: {:?}", response);
}
