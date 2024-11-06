use tower_lsp::lsp_types::*;

use std::path::PathBuf;

pub enum MsgToServer {
    ComposerFiles(Vec<PathBuf>),
    DidOpen {
        url: Url,
        text: String,
        version: i32,
    },
    DidChange {
        url: Url,
        content_changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    },
    DocumentSymbol(Url),
    Shutdown,
}

pub enum MsgFromServer {
    References(Vec<Location>),
    NestedSymbols(Vec<DocumentSymbol>),
}
