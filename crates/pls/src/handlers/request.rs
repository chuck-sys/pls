use crossbeam_channel::SendError;

use lsp_server::{Connection, Message, RequestId, Response};

use lsp_types::*;

use crate::global_state::GlobalState;

fn send_ok<T: serde::Serialize>(connection: &Connection, id: RequestId, result: &T) -> Result<(), SendError<Message>> {
    let response = Response {
        id,
        result: Some(serde_json::to_value(result).unwrap()),
        error: None,
    };

    connection.sender.send(Message::Response(response)).map(|_| ())
}

fn send_err<T: serde::Serialize>(connection: &Connection, id: RequestId, code: lsp_server::ErrorCode, msg: &str) -> Result<(), SendError<Message>> {
    let response = Response {
        id,
        result: None,
        error: Some(lsp_server::ResponseError {
            code: code as i32,
            message: msg.into(),
            data: None,
        }),
    };

    connection.sender.send(Message::Response(response)).map(|_| ())
}

pub fn code_action(
    state: &mut GlobalState,
    (request_id, params): (RequestId, CodeActionParams),
) -> Result<(), std::convert::Infallible> {
    let actions: CodeActionResponse = Vec::new();

    let _ = send_ok(&state.connection, request_id, &actions);

    Ok(())
}
