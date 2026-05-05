use lsp_types::DidOpenTextDocumentParams;

use crate::global_state::GlobalState;

pub fn did_open_text_document(
    state: &mut GlobalState,
    params: DidOpenTextDocumentParams,
) -> anyhow::Result<()> {
    todo!()
}
