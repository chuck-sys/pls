use tower_lsp::Client;
use tower_lsp::lsp_types::*;

use async_channel::{Receiver, Sender};

use crate::msg::{MsgFromServer, MsgToServer};

pub struct Server {
    pub client: Client,
    pub sender_to_backend: Sender<MsgFromServer>,
    pub receiver_from_backend: Receiver<MsgToServer>,
}

impl Server {
    pub async fn serve(&mut self) {
        self.client.log_message(MessageType::LOG, "starting to serve");

        loop {
            match self.receiver_from_backend.recv_blocking() {
                Ok(msg) => match msg {
                    MsgToServer::Shutdown => break,
                    _ => unimplemented!(),
                },
                Err(e) => self.client.log_message(MessageType::ERROR, e).await,
            }
        }
    }
}
