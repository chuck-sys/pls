use crossbeam_channel::SendError;
use lsp_server::{Connection, Message, RequestId, Response};
use lsp_types::*;
use pls_types::UriExt as _;
use serde_json::json;

use crate::code_action::{TMPLSTR_TITLE, PHPECHO_TITLE, can_change_to_tmplstr};
use crate::global_state::GlobalState;

fn send_ok<T: serde::Serialize>(
    connection: &Connection,
    id: RequestId,
    result: &T,
) -> Result<(), SendError<Message>> {
    let response = Response {
        id,
        result: Some(serde_json::to_value(result).unwrap()),
        error: None,
    };

    connection
        .sender
        .send(Message::Response(response))
        .map(|_| ())
}

fn send_err<T: serde::Serialize>(
    connection: &Connection,
    id: RequestId,
    code: lsp_server::ErrorCode,
    msg: &str,
) -> Result<(), SendError<Message>> {
    let response = Response {
        id,
        result: None,
        error: Some(lsp_server::ResponseError {
            code: code as i32,
            message: msg.into(),
            data: None,
        }),
    };

    connection
        .sender
        .send(Message::Response(response))
        .map(|_| ())
}

pub fn code_action(
    request_id: RequestId,
    state: &mut GlobalState,
    params: CodeActionParams,
) -> anyhow::Result<()> {
    let mut actions: CodeActionResponse = Vec::new();

    if let Some(file_name) = params
        .text_document
        .uri
        .to_file_path()
        .map(|x| x.to_path_buf())
    {
        if let Some(file_info) = state.file_infos.get(&file_name) {
            if params.range.start == params.range.end {
                if file_info.content.contains("<?php echo ") {
                    actions.push(
                        CodeAction {
                            title: PHPECHO_TITLE.to_string(),
                            kind: Some(CodeActionKind::SOURCE),
                            data: Some(json!({"uri": params.text_document.uri})),
                            ..CodeAction::default()
                        }
                        .into(),
                    );
                }
            }

            if can_change_to_tmplstr(file_info, &params.range) {
                actions.push(
                    CodeAction {
                        title: TMPLSTR_TITLE.to_string(),
                        kind: Some(CodeActionKind::SOURCE),
                        data: Some(json!({"uri": params.text_document.uri})),
                        ..CodeAction::default()
                    }
                    .into(),
                );
            }
        }
    }

    let _ = send_ok(&state.connection, request_id, &actions);

    Ok(())
}

pub fn code_action_resolve(
    request_id: RequestId,
    state: &mut GlobalState,
    params: CodeAction,
) -> anyhow::Result<()> {
    match (params.title.as_ref(), params.data) {
        (PHPECHO_TITLE, Some(v)) => {
            let v: crate::code_action::PhpEchoParams = serde_json::from_value(v)?;
            let file_name = v
                .uri
                .to_file_path()
                .ok_or(anyhow::anyhow!("cannot convert uri to path"))?
                .to_path_buf();
            let file_info = state
                .file_infos
                .get(&file_name)
                .ok_or(anyhow::anyhow!("file `{file_name:?}` not loaded"))?;
            let document_changes =
                crate::code_action::changes_phpecho(&v.uri, &file_info.content, file_info.version);

            let _ = send_ok(
                &state.connection,
                request_id,
                &CodeAction {
                    title: PHPECHO_TITLE.to_string(),
                    kind: Some(CodeActionKind::SOURCE),
                    edit: Some(WorkspaceEdit {
                        document_changes,
                        ..WorkspaceEdit::default()
                    }),
                    ..CodeAction::default()
                },
            );
        }
        _ => {}
    }

    Ok(())
}
