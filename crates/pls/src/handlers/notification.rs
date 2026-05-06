use lsp_types::DidOpenTextDocumentParams;
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
