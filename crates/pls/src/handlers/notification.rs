use crate::global_state::GlobalState;

use lsp_types::DidOpenTextDocumentParams;

pub fn did_open_text_document(
    state: &mut GlobalState,
    params: DidOpenTextDocumentParams,
) -> Result<(), std::convert::Infallible> {
    Ok(())
}
