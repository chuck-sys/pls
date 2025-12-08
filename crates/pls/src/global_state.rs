use crossbeam_channel::{Receiver, Sender};

use crate::config::Config;
use crate::messages::AnalysisThreadMessage;

/// Inspired by `rust-analyzer`
pub struct GlobalState {
    pub cfg: Config,
    pub connection: lsp_server::Connection,

    pub analysis_send: Sender<AnalysisThreadMessage>,
    pub analysis_recv: Receiver<()>,

    pub running: bool,
}
