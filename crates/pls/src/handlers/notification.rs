use lsp_types::{DidChangeTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams};
use pls_types::UriExt;

use crate::file::parse;
use crate::global_state::{FileInfo, GlobalState};
use crate::messages::Task;

pub fn did_save_text_document(
    state: &mut GlobalState,
    params: DidSaveTextDocumentParams,
) -> anyhow::Result<()> {
    let file_name = params.text_document.uri.to_file_path().ok_or(anyhow::anyhow!("file name -> pathbuf conversion"))?.to_path_buf();
    let content = params.text.ok_or(anyhow::anyhow!("no text content even though it was configured"))?;
    let version = 0;

    let (php_ast, phpdoc_ast) = parse(&content, (None, None));

    state.file_infos.insert(file_name.clone(), FileInfo {
        file_name: file_name.clone(),
        content,
        version,
        php_ast,
        phpdoc_ast,
    });

    state.worker_send.send(Task::AnalyzeFile(file_name))?;

    Ok(())
}

pub fn did_open_text_document(
    state: &mut GlobalState,
    params: DidOpenTextDocumentParams,
) -> anyhow::Result<()> {
    let file_name = params.text_document.uri.to_file_path().ok_or(anyhow::anyhow!("file name -> pathbuf conversion"))?.to_path_buf();
    let content = params.text_document.text;
    let version = params.text_document.version;

    let (php_ast, phpdoc_ast) = parse(&content, (None, None));

    state.file_infos.insert(file_name.clone(), FileInfo {
        file_name: file_name.clone(),
        content,
        version,
        php_ast,
        phpdoc_ast,
    });

    state.worker_send.send(Task::AnalyzeFile(file_name))?;

    Ok(())
}

pub fn did_change_text_document(
    state: &mut GlobalState,
    params: DidChangeTextDocumentParams,
) -> anyhow::Result<()> {
    let file_name = params.text_document.uri.to_file_path().ok_or(anyhow::anyhow!("file name -> pathbuf conversion"))?.to_path_buf();
    let file_info = state.file_infos.get_mut(&file_name).ok_or(anyhow::anyhow!("file change when not opened"))?;

    if file_info.version >= params.text_document.version {
        return Err(anyhow::anyhow!("blocking document change that has non-update version"));
    }

    for c in params.content_changes {
        match file_info.change(c) {
            Err(e) => log::error!("could not execute a document change because: {e}"),
            _ => {}
        }
    }

    // FIXME handle errors when you execute document changes
    (file_info.php_ast, file_info.phpdoc_ast) = parse(&file_info.content, (Some(&file_info.php_ast), Some(&file_info.phpdoc_ast)));

    state.worker_send.send(Task::AnalyzeFile(file_name))?;

    Ok(())
}
