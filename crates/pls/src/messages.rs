use tower_lsp_server::lsp_types::Uri;

use crate::php_namespace::PhpNamespace;

pub enum AnalysisThreadMessage {
    Shutdown,
    AnalyzeUri(Uri),
    AnalyzeNs(PhpNamespace),
}

pub enum AnalysisThreadQueueItem {
    Uri(Uri),
    Ns(PhpNamespace),
}
