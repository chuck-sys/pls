use crossbeam_channel::{Sender, Receiver};

use crate::config::Config;

/// Inspired by `rust-analyzer`
pub struct GlobalState {
    cfg: Config,

    send: Sender<lsp_server::Message>,
    recv: Receiver<lsp_server::Message>,
}
