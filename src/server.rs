use tower_lsp::Client;
use tower_lsp::lsp_types::*;

use async_channel::{Receiver, Sender};

use tree_sitter::{Parser, Tree, Node};

use std::collections::HashMap;

use crate::msg::{MsgFromServer, MsgToServer};

struct FileData {
    contents: String,
    tree: Tree,
}

pub struct Server {
    client: Client,
    sender_to_backend: Sender<MsgFromServer>,
    receiver_from_backend: Receiver<MsgToServer>,
    parser: Parser,

    file_trees: HashMap<Url, FileData>,
}

fn range_plaintext(file_contents: &String, range: tree_sitter::Range) -> String {
    file_contents[range.start_byte..range.end_byte].to_owned()
}

fn to_position(point: &tree_sitter::Point) -> Position {
    Position {
        line: point.row as u32,
        character: point.column as u32,
    }
}

fn to_range(range: &tree_sitter::Range) -> Range {
    Range {
        start: to_position(&range.start_point),
        end: to_position(&range.end_point),
    }
}

fn document_symbols_property_decl(uri: &Url, property_node: &Node, file_contents: &String, ret: &mut Vec<SymbolInformation>) {
    let mut cursor = property_node.walk();
    if !cursor.goto_first_child() {
        return;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "property_element" {
            cursor.goto_first_child();

            ret.push(SymbolInformation {
                name: range_plaintext(file_contents, cursor.node().range()),
                kind: SymbolKind::PROPERTY,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: to_range(&property_node.range()),
                },
                container_name: None,
            });

            return;
        }

        if !cursor.goto_next_sibling() {
            return;
        }
    }
}

fn document_symbols_class_decl(uri: &Url, class_node: &Node, file_contents: &String, ret: &mut Vec<SymbolInformation>) {
    if let Some(name_node) = class_node.child_by_field_name("name") {
        ret.push(SymbolInformation {
            name: range_plaintext(file_contents, name_node.range()),
            kind: SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range: to_range(&class_node.range()),
            },
            container_name: None,
        });
    }

    if let Some(decl_list) = class_node.child_by_field_name("body") {
        let mut cursor = decl_list.walk();
        if !cursor.goto_first_child() {
            return;
        }

        loop {
            let kind = cursor.node().kind();
            if kind == "property_declaration" {
                document_symbols_property_decl(uri, &cursor.node(), file_contents, ret);
            } else if kind == "{" || kind == "}" {
                // ignore these
            } else if kind == "method_declaration" {
                // unimpl
            } else {
                unimplemented!("{}", kind);
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn document_symbols(uri: &Url, root_node: &Node, file_contents: &String) -> Vec<SymbolInformation> {
    let mut ret = Vec::new();
    let mut cursor = root_node.walk();

    if !cursor.goto_first_child() {
        return ret;
    }

    loop {
        let kind = cursor.node().kind();
        // DFS
        if kind == "class_declaration" {
            document_symbols_class_decl(uri, &cursor.node(), file_contents, &mut ret);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }

    ret
}

impl Server {
    pub fn new(client: Client, sx: Sender<MsgFromServer>, rx: Receiver<MsgToServer>) -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_php::language_php()).expect("error loading PHP grammar");

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
                    MsgToServer::DocumentSymbol(url) => self.document_symbol(url).await,
                    _ => unimplemented!(),
                },
                Err(e) => self.client.log_message(MessageType::ERROR, e).await,
            }
        }
    }

    async fn did_open(&mut self, url: Url, text: String, version: i32) {
        match self.parser.parse(&text, None) {
            Some(tree) => {
                self.file_trees.insert(url, FileData { contents: text, tree });
            },
            None => self.client.log_message(MessageType::ERROR, format!("could not parse file `{}`", &url)).await,
        }
    }

    async fn document_symbol(&mut self, url: Url) {
        if let Some(FileData {contents, tree}) = self.file_trees.get(&url) {
            if let Err(e) = self.sender_to_backend.send(MsgFromServer::FlatSymbols(document_symbols(&url, &tree.root_node(), contents))).await {
                self.client.log_message(MessageType::ERROR, format!("document_symbol: unable to send to backend: {}", e)).await;
            }
        } else {
            if let Err(e) = self.sender_to_backend.send(MsgFromServer::FlatSymbols(vec![])).await {
                self.client.log_message(MessageType::ERROR, format!("document_symbol: unable to send; no file `{}`: {}", &url, e)).await;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use tree_sitter::Parser;
    use tower_lsp::lsp_types::*;

    use super::document_symbols;

    #[test]
    fn test_get_symbols() {
        let source = "<?php
            class Whatever {
                public int $x = 12;
                public function foo(int $bar): void
                {
                    $this->x = $bar;
                }
            }";
        let expected_symbols = ["Whatever", "$x", "foo", "$bar"];
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_php::language_php()).expect("error loading PHP grammar");

        let tree = parser.parse(source, None).unwrap();
        let root_node = tree.root_node();
        let uri = Url::from_file_path("/home/file.php").unwrap();
        let actual_symbols: Vec<String> = document_symbols(&uri, &root_node, &source.to_string())
            .into_iter()
            .map(|SymbolInformation { name, .. }| name)
            .collect();
        assert_eq!(actual_symbols, expected_symbols);
    }
}
