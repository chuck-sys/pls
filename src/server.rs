use tower_lsp::lsp_types::*;
use tower_lsp::Client;

use async_channel::{Receiver, Sender};

use tree_sitter::{Node, Parser, Tree};

use std::collections::HashMap;
use std::path::PathBuf;

use crate::msg::{MsgFromServer, MsgToServer};
use crate::php_namespace::PhpNamespace;

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
    namespace_to_dir: HashMap<PhpNamespace, Vec<PathBuf>>,
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

fn document_symbols_property_decl(
    uri: &Url,
    property_node: &Node,
    file_contents: &String,
) -> Option<DocumentSymbol> {
    let mut cursor = property_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "property_element" {
            cursor.goto_first_child();

            return Some(DocumentSymbol {
                name: range_plaintext(file_contents, cursor.node().range()),
                detail: Some(range_plaintext(file_contents, property_node.range())),
                kind: SymbolKind::PROPERTY,
                tags: None,
                deprecated: None,
                range: to_range(&cursor.node().range()),
                selection_range: to_range(&property_node.range()),
                children: None,
            });
        }

        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn document_symbols_method_params_decl(
    uri: &Url,
    params: &Node,
    file_contents: &String,
) -> Vec<DocumentSymbol> {
    let mut symbols = vec![];
    let mut cursor = params.walk();
    if !cursor.goto_first_child() {
        return symbols;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "simple_parameter" {
            if let Some(name_node) = cursor.node().child_by_field_name("name") {
                symbols.push(DocumentSymbol {
                    name: range_plaintext(file_contents, name_node.range()),
                    detail: Some(range_plaintext(file_contents, cursor.node().range())),
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    deprecated: None,
                    range: to_range(&name_node.range()),
                    selection_range: to_range(&cursor.node().range()),
                    children: None,
                });
            }
        }

        if !cursor.goto_next_sibling() {
            return symbols;
        }
    }
}

fn document_symbols_method_decl(
    uri: &Url,
    method_node: &Node,
    file_contents: &String,
) -> Vec<DocumentSymbol> {
    let mut symbols = vec![];

    if let Some(method_parameters_node) = method_node.child_by_field_name("parameters") {
        symbols.extend(document_symbols_method_params_decl(uri, &method_parameters_node, file_contents));
    }

    symbols
}

fn document_symbols_class_decl(
    uri: &Url,
    class_node: &Node,
    file_contents: &String,
) -> Vec<DocumentSymbol> {
    let mut symbols = vec![];

    if let Some(decl_list) = class_node.child_by_field_name("body") {
        let mut cursor = decl_list.walk();
        if !cursor.goto_first_child() {
            return symbols;
        }

        loop {
            let kind = cursor.node().kind();
            if kind == "property_declaration" {
                if let Some(prop_docsym) = document_symbols_property_decl(uri, &cursor.node(), file_contents) {
                    symbols.push(prop_docsym);
                }
            } else if kind == "{" || kind == "}" {
                // ignore these
            } else if kind == "method_declaration" {
                if let Some(name_node) = cursor.node().child_by_field_name("name") {
                    let children = document_symbols_method_decl(uri, &cursor.node(), file_contents);
                    let method_name = range_plaintext(file_contents, name_node.range());
                    let kind = if &method_name == "__constructor" { SymbolKind::CONSTRUCTOR } else { SymbolKind::METHOD };
                    symbols.push(DocumentSymbol {
                        name: method_name,
                        detail: None,
                        kind,
                        tags: None,
                        deprecated: None,
                        range: to_range(&name_node.range()),
                        selection_range: to_range(&cursor.node().range()),
                        children: Some(children),
                    });
                }
            } else {
                unimplemented!("{}", kind);
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    symbols
}

fn document_symbols(uri: &Url, root_node: &Node, file_contents: &String) -> Vec<DocumentSymbol> {
    let mut ret = Vec::new();
    let mut cursor = root_node.walk();

    if !cursor.goto_first_child() {
        return ret;
    }

    loop {
        let kind = cursor.node().kind();
        // DFS
        if kind == "class_declaration" {
            if let Some(name_node) = cursor.node().child_by_field_name("name") {
                let children = document_symbols_class_decl(uri, &cursor.node(), file_contents);
                ret.push(DocumentSymbol {
                    name: range_plaintext(file_contents, name_node.range()),
                    detail: None,
                    kind: SymbolKind::CLASS,
                    tags: None,
                    deprecated: None,
                    range: to_range(&name_node.range()),
                    selection_range: to_range(&cursor.node().range()),
                    children: Some(children),
                });
            }
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
        parser
            .set_language(&tree_sitter_php::language_php())
            .expect("error loading PHP grammar");

        Self {
            client,
            sender_to_backend: sx,
            receiver_from_backend: rx,
            parser,

            file_trees: HashMap::new(),
            namespace_to_dir: HashMap::new(),
        }
    }

    pub async fn serve(&mut self) {
        self.client
            .log_message(MessageType::LOG, "starting to serve")
            .await;

        loop {
            match self.receiver_from_backend.recv_blocking() {
                Ok(msg) => match msg {
                    MsgToServer::Shutdown => break,
                    MsgToServer::DidOpen { url, text, version } => {
                        self.did_open(url, text, version).await
                    }
                    MsgToServer::DocumentSymbol(url) => self.document_symbol(url).await,
                    MsgToServer::ComposerFiles(composer_files) => {
                        self.read_composer_files(composer_files).await
                    }
                },
                Err(e) => self.client.log_message(MessageType::ERROR, e).await,
            }
        }
    }

    async fn read_composer_files(&mut self, composer_files: Vec<PathBuf>) {
        composer_files.iter().for_each(|file| {
            if !file.exists() {
                return;
            }
        });
    }

    async fn did_open(&mut self, url: Url, text: String, version: i32) {
        match self.parser.parse(&text, None) {
            Some(tree) => {
                self.file_trees.insert(
                    url,
                    FileData {
                        contents: text,
                        tree,
                    },
                );
            }
            None => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("could not parse file `{}`", &url),
                    )
                    .await
            }
        }
    }

    async fn document_symbol(&mut self, url: Url) {
        if let Some(FileData { contents, tree }) = self.file_trees.get(&url) {
            if let Err(e) = self
                .sender_to_backend
                .send(MsgFromServer::NestedSymbols(document_symbols(
                    &url,
                    &tree.root_node(),
                    contents,
                )))
                .await
            {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("document_symbol: unable to send to backend: {}", e),
                    )
                    .await;
            }
        } else {
            if let Err(e) = self
                .sender_to_backend
                .send(MsgFromServer::NestedSymbols(vec![]))
                .await
            {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("document_symbol: unable to send; no file `{}`: {}", &url, e),
                    )
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use tower_lsp::lsp_types::*;
    use tree_sitter::Parser;

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

                public function fee(string $sound, ?array $down): int|false
                {
                    $this->x = 12;
                    if (!empty($down)) {
                        $this->x = ((int) $sound) + ((int) $down[0]);
                    }
                }
            }

            final class Another {
                private int $y = 3;
                public function __constructor(): void
                {
                }
            }";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::language_php())
            .expect("error loading PHP grammar");

        let tree = parser.parse(source, None).unwrap();
        let root_node = tree.root_node();
        let uri = Url::from_file_path("/home/file.php").unwrap();
        let actual_symbols = document_symbols(&uri, &root_node, &source.to_string());
        assert_eq!(2, actual_symbols.len());
        assert_eq!("Whatever", &actual_symbols[0].name);
        assert_eq!("Another", &actual_symbols[1].name);
        assert_eq!(3, actual_symbols[0].children.as_ref().unwrap().len());
        assert_eq!("$x", &actual_symbols[0].children.as_ref().unwrap()[0].name);
        assert_eq!("foo", &actual_symbols[0].children.as_ref().unwrap()[1].name);
        assert_eq!("fee", &actual_symbols[0].children.as_ref().unwrap()[2].name);
        assert_eq!(1, actual_symbols[0].children.as_ref().unwrap()[1].children.as_ref().unwrap().len());
        assert_eq!("$bar", &actual_symbols[0].children.as_ref().unwrap()[1].children.as_ref().unwrap()[0].name);
        assert_eq!(2, actual_symbols[0].children.as_ref().unwrap()[2].children.as_ref().unwrap().len());
        assert_eq!("$sound", &actual_symbols[0].children.as_ref().unwrap()[2].children.as_ref().unwrap()[0].name);
        assert_eq!("$down", &actual_symbols[0].children.as_ref().unwrap()[2].children.as_ref().unwrap()[1].name);
        assert_eq!("?array $down", actual_symbols[0].children.as_ref().unwrap()[2].children.as_ref().unwrap()[1].detail.as_ref().unwrap());
        assert_eq!(2, actual_symbols[1].children.as_ref().unwrap().len());
        assert_eq!("private int $y = 3;", actual_symbols[1].children.as_ref().unwrap()[0].detail.as_ref().unwrap());
        assert_eq!(SymbolKind::CONSTRUCTOR, actual_symbols[1].children.as_ref().unwrap()[1].kind);
    }
}
