use lsp_types::Uri;

use pls_types::PhpNamespace;

pub enum AnalysisThreadMessage {
    Shutdown,
    AnalyzeUri(Uri),
    AnalyzeNs(PhpNamespace),
}

pub enum AnalysisThreadQueueItem {
    Uri(Uri),
    Ns(PhpNamespace),
}
