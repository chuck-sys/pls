use lsp_types::Uri;

use std::path::PathBuf;

#[derive(Debug)]
pub enum Task {
    AnalyzeStubs,
    AnalyzeFile(PathBuf),
}

pub enum AnalysisThreadMessage {
    AnalyzeUri(Uri),
}

pub enum AnalysisThreadQueueItem {
    Uri(Uri),
}
