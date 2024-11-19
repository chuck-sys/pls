use tower_lsp::lsp_types::*;

use std::path::PathBuf;
use std::fmt;

#[derive(Clone)]
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

impl fmt::Display for MsgToServer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MsgToServer::ComposerFiles(_) => write!(f, "ComposerFiles"),
            MsgToServer::DidOpen { url, text, version } => write!(f, "DidOpen"),
            MsgToServer::DidChange { url, content_changes, version } => write!(f, "DidChange"),
            MsgToServer::DocumentSymbol(_) => write!(f, "DocumentSymbol"),
            MsgToServer::Shutdown => write!(f, "Shutdown"),
        }
    }
}

pub enum MsgFromServer {
    References(Vec<Location>),
    NestedSymbols(Vec<DocumentSymbol>),
}
