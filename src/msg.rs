use tower_lsp::lsp_types::*;

pub enum MsgToServer {
    DidOpen {
        url: Url,
        text: String,
        version: i32,
    },
    DocumentSymbol(Url),
    Shutdown,
}

pub enum MsgFromServer {
    References(Vec<Location>),
    FlatSymbols(Vec<SymbolInformation>),
    NestedSymbols(Vec<DocumentSymbol>),
}
