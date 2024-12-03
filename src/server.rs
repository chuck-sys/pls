use tower_lsp::lsp_types::*;
use tower_lsp::Client;

use async_channel::{Receiver, Sender};

use tree_sitter::{InputEdit, Node, Parser, Tree};

use std::collections::HashMap;
use std::path::PathBuf;
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;

use crate::msg::{MsgFromServer, MsgToServer};
use crate::php_namespace::PhpNamespace;

struct FileData {
    contents: String,
    tree: Tree,
    version: i32,
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

fn to_point(position: &Position) -> tree_sitter::Point {
    tree_sitter::Point {
        row: position.line as usize,
        column: position.character as usize,
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
        symbols.extend(document_symbols_method_params_decl(
            uri,
            &method_parameters_node,
            file_contents,
        ));
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
                if let Some(prop_docsym) =
                    document_symbols_property_decl(uri, &cursor.node(), file_contents)
                {
                    symbols.push(prop_docsym);
                }
            } else if kind == "{" || kind == "}" || kind == "comment" {
                // ignore these
            } else if kind == "method_declaration" {
                if let Some(name_node) = cursor.node().child_by_field_name("name") {
                    let children = document_symbols_method_decl(uri, &cursor.node(), file_contents);
                    let method_name = range_plaintext(file_contents, name_node.range());
                    let kind = if &method_name == "__constructor" {
                        SymbolKind::CONSTRUCTOR
                    } else {
                        SymbolKind::METHOD
                    };
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

/// Get byte offset given some row and column position in a file.
///
/// For example, line 1 character 1 should have offset of 0 (0-indexing).
///
/// Return None if the position is invalid (i.e. not in the file, out of range of current line,
/// etc.)
fn byte_offset(text: &String, r: &Position) -> Option<usize> {
    if r.character == 0 {
        return None;
    }

    let line = r.line as usize;
    // start on the zeroth, not the first, because that's how offsets work
    let character = r.character as usize - 1;
    let mut current_offset = 0usize;

    for (line_text, line_num) in text.lines().zip(1..=line) {
        if line_num == line {
            if character > line_text.len() {
                return None;
            } else {
                return Some(current_offset + character);
            }
        } else {
            let newline_offset = current_offset + line_text.len();
            // assume only two types of newlines exist: `\n` and `\r\n`
            let newline_num_bytes = if text[newline_offset..].starts_with("\n") {
                1
            } else {
                2
            };

            current_offset += line_text.len() + newline_num_bytes;
        }
    }

    None
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
        loop {
            match self.receiver_from_backend.recv().await {
                Ok(msg) => match msg {
                    MsgToServer::Shutdown => break,
                    MsgToServer::DidOpen { url, text, version } => {
                        self.did_open(url, text, version).await
                    }
                    MsgToServer::DidChange {
                        url,
                        content_changes,
                        version,
                    } => self.did_change(url, content_changes, version).await,
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
        for path in composer_files {
            if !path.exists() {
                return;
            }

            let mut task = |path| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                let file = File::open(path)?;
                let reader = BufReader::new(file);

                let v: serde_json::Value = serde_json::from_reader(reader)?;
                if let serde_json::Value::Object(autoload) = &v["autoload"] {
                    if let serde_json::Value::Object(psr4) = &autoload["psr-4"] {
                        for (ns, dir) in psr4 {
                            let namespace = PhpNamespace::from_str(ns).unwrap();
                            match dir {
                                serde_json::Value::Array(dirs) => {
                                    let mut paths = vec![];
                                    for x in dirs {
                                        if let serde_json::Value::String(dir) = x {
                                            if let Ok(path) = PathBuf::from_str(dir) {
                                                paths.push(path);
                                            }
                                        }
                                    }

                                    if paths.len() > 0 {
                                        self.namespace_to_dir.insert(namespace, paths);
                                    }
                                },
                                serde_json::Value::String(dir) => {
                                    let dir = PathBuf::from_str(dir)?;
                                    self.namespace_to_dir.insert(namespace, vec![dir]);
                                },
                                _ => {},
                            }
                        }
                    }

                    if let serde_json::Value::Object(psr0) = &autoload["psr-0"] {
                    }

                    if let serde_json::Value::Array(files) = &autoload["files"] {
                    }
                }

                Ok(())
            };

            if let Err(e) = task(path) {
                self.client.log_message(MessageType::ERROR, e).await;
            }
        }
    }

    async fn did_open(&mut self, url: Url, text: String, version: i32) {
        match self.parser.parse(&text, None) {
            Some(tree) => {
                self.file_trees.insert(
                    url,
                    FileData {
                        contents: text,
                        tree,
                        version,
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

    async fn did_change(
        &mut self,
        url: Url,
        content_changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) {
        match self.file_trees.get_mut(&url) {
            Some(entry) => {
                if entry.version >= version {
                    self.client
                        .log_message(
                            MessageType::LOG,
                            format!("didChange tried to change same version for file `{}`", &url),
                        )
                        .await;
                    return;
                }

                entry.version = version;
                for change in content_changes {
                    if let Some(r) = change.range {
                        if let (Some(start_byte), Some(end_byte)) = (
                            byte_offset(&change.text, &r.start),
                            byte_offset(&change.text, &r.end),
                        ) {
                            let input_edit = InputEdit {
                                start_byte,
                                old_end_byte: end_byte,
                                new_end_byte: change.text.len(),
                                start_position: to_point(&r.start),
                                old_end_position: to_point(&r.end),
                                new_end_position: {
                                    let mut row = r.start.line as usize;
                                    let mut column = r.start.character as usize;

                                    for c in change.text.chars() {
                                        if c == '\n' {
                                            row += 1;
                                            column = 0;
                                        } else {
                                            column += 1;
                                        }
                                    }

                                    tree_sitter::Point { row, column }
                                },
                            };
                            entry.tree.edit(&input_edit);
                            entry
                                .contents
                                .replace_range(start_byte..end_byte, &change.text);
                        }
                    } else {
                        entry.contents = change.text.clone();
                    }

                    match self.parser.parse(&entry.contents, None) {
                        Some(tree) => {
                            entry.tree = tree;
                        }
                        None => {
                            self.client
                                .log_message(MessageType::ERROR, "could not parse change")
                                .await;
                        }
                    }
                }
            }
            None => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!(
                            "didChange event triggered without didOpen for file `{}`",
                            &url
                        ),
                    )
                    .await;
            }
        }
    }

    async fn document_symbol(&mut self, url: Url) {
        if let Some(FileData { contents, tree, .. }) = self.file_trees.get(&url) {
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

    use super::byte_offset;
    use super::document_symbols;

    const SOURCE: &'static str = "<?php
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

    #[test]
    fn test_valid_byte_offsets() {
        let valids = [
            (
                Position {
                    line: 1,
                    character: 1,
                },
                0usize,
            ),
            (
                Position {
                    line: 2,
                    character: 1,
                },
                6usize,
            ),
        ];

        let s = SOURCE.to_string();
        for (pos, expected) in valids {
            assert_eq!(expected, byte_offset(&s, &pos).unwrap());
        }
    }

    #[test]
    fn test_invalid_byte_offsets() {
        let invalids = [
            Position {
                line: 200,
                character: 10,
            },
            Position {
                line: 1,
                character: 100,
            },
        ];

        let s = SOURCE.to_string();
        for invalid_position in invalids {
            assert_eq!(None, byte_offset(&s, &invalid_position));
        }
    }

    #[test]
    fn test_get_symbols() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::language_php())
            .expect("error loading PHP grammar");

        let tree = parser.parse(SOURCE, None).unwrap();
        let root_node = tree.root_node();
        let uri = Url::from_file_path("/home/file.php").unwrap();
        let actual_symbols = document_symbols(&uri, &root_node, &SOURCE.to_string());
        assert_eq!(2, actual_symbols.len());
        assert_eq!("Whatever", &actual_symbols[0].name);
        assert_eq!("Another", &actual_symbols[1].name);
        assert_eq!(3, actual_symbols[0].children.as_ref().unwrap().len());
        assert_eq!("$x", &actual_symbols[0].children.as_ref().unwrap()[0].name);
        assert_eq!("foo", &actual_symbols[0].children.as_ref().unwrap()[1].name);
        assert_eq!("fee", &actual_symbols[0].children.as_ref().unwrap()[2].name);
        assert_eq!(
            1,
            actual_symbols[0].children.as_ref().unwrap()[1]
                .children
                .as_ref()
                .unwrap()
                .len()
        );
        assert_eq!(
            "$bar",
            &actual_symbols[0].children.as_ref().unwrap()[1]
                .children
                .as_ref()
                .unwrap()[0]
                .name
        );
        assert_eq!(
            2,
            actual_symbols[0].children.as_ref().unwrap()[2]
                .children
                .as_ref()
                .unwrap()
                .len()
        );
        assert_eq!(
            "$sound",
            &actual_symbols[0].children.as_ref().unwrap()[2]
                .children
                .as_ref()
                .unwrap()[0]
                .name
        );
        assert_eq!(
            "$down",
            &actual_symbols[0].children.as_ref().unwrap()[2]
                .children
                .as_ref()
                .unwrap()[1]
                .name
        );
        assert_eq!(
            "?array $down",
            actual_symbols[0].children.as_ref().unwrap()[2]
                .children
                .as_ref()
                .unwrap()[1]
                .detail
                .as_ref()
                .unwrap()
        );
        assert_eq!(2, actual_symbols[1].children.as_ref().unwrap().len());
        assert_eq!(
            "private int $y = 3;",
            actual_symbols[1].children.as_ref().unwrap()[0]
                .detail
                .as_ref()
                .unwrap()
        );
        assert_eq!(
            SymbolKind::CONSTRUCTOR,
            actual_symbols[1].children.as_ref().unwrap()[1].kind
        );
    }
}
