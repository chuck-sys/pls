use lsp_types::Uri;

pub enum AnalysisThreadMessage {
    Shutdown,
    AnalyzeUri(Uri),
}

pub enum AnalysisThreadQueueItem {
    Uri(Uri),
}
