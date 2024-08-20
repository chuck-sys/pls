use tower_lsp::Client;
use tower_lsp::lsp_types::*;

use async_channel::{Receiver, Sender};

use tree_sitter::{Language, Parser, Tree};

use std::collections::HashMap;

use crate::msg::{MsgFromServer, MsgToServer};

pub struct Server {
    client: Client,
    sender_to_backend: Sender<MsgFromServer>,
    receiver_from_backend: Receiver<MsgToServer>,
    parser: Parser,

    file_trees: HashMap<Url, Tree>,
}

impl Server {
    pub fn new(client: Client, sx: Sender<MsgFromServer>, rx: Receiver<MsgToServer>) -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_php::language_php()).expect("error loadnig PHP grammar");

        Self {
            client,
            sender_to_backend: sx,
            receiver_from_backend: rx,
            parser,

            file_trees: HashMap::new(),
        }
    }

    pub async fn serve(&mut self) {
        self.client.log_message(MessageType::LOG, "starting to serve").await;

        loop {
            match self.receiver_from_backend.recv_blocking() {
                Ok(msg) => match msg {
                    MsgToServer::Shutdown => break,
                    MsgToServer::DidOpen { url, text, version } => self.did_open(url, text, version).await,
                    _ => unimplemented!(),
                },
                Err(e) => self.client.log_message(MessageType::ERROR, e).await,
            }
        }
    }

    async fn did_open(&mut self, url: Url, text: String, version: i32) {
        match self.parser.parse(text, None) {
            Some(tree) => {
                self.file_trees.insert(url, tree);
            },
            None => self.client.log_message(MessageType::ERROR, format!("could not parse file `{}`", &url)).await,
        }
    }
}
