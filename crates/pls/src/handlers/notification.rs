use lsp_types::{DidChangeTextDocumentParams, DidOpenTextDocumentParams};
use pls_types::UriExt;

use crate::global_state::{FileInfo, GlobalState};

pub fn did_open_text_document(
    state: &mut GlobalState,
    params: DidOpenTextDocumentParams,
) -> anyhow::Result<()> {
    let file_name = params.text_document.uri.to_file_path().ok_or(anyhow::anyhow!("file name -> pathbuf conversion"))?.to_path_buf();
    let content = params.text_document.text;
    let version = params.text_document.version;
    let ast = state.parsers.parse(&content, None).ok_or(anyhow::anyhow!("parser failure"))?;

    state.file_infos.insert(file_name.clone(), FileInfo {
        file_name,
        content,
        version,
        ast,
    });

    Ok(())
}

pub fn did_change_text_document(
    state: &mut GlobalState,
    params: DidChangeTextDocumentParams,
) -> anyhow::Result<()> {
    let file_name = params.text_document.uri.to_file_path().ok_or(anyhow::anyhow!("file name -> pathbuf conversion"))?.to_path_buf();
    let mut file_info = state.file_infos.get_mut(&file_name).ok_or(anyhow::anyhow!("file change when not opened"))?;

    if file_info.version >= params.text_document.version {
        return Err(anyhow::anyhow!("blocking document change that has non-update version"));
    }

    todo!("actually implement did_change by how we did so previously");

    Ok(())
}
