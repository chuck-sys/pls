use crossbeam_channel::{Sender, Receiver};

use crate::config::Config;

/// Inspired by `rust-analyzer`
pub struct GlobalState {
    pub cfg: Config,

    pub send: Sender<lsp_server::Message>,
    pub recv: Receiver<lsp_server::Message>,
}
