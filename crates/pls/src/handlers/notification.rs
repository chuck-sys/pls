use lsp_types::DidOpenTextDocumentParams;

pub fn did_open_text_document<Db: salsa::Database>(
    params: DidOpenTextDocumentParams,
) -> Result<(), std::convert::Infallible> {
    todo!()
}
