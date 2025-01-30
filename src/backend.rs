use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use tree_sitter::{InputEdit, Node, Parser, Tree};

use tokio::sync::RwLock;

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::str::FromStr;

use crate::php_namespace::PhpNamespace;

struct FileData {
    contents: String,
    tree: Tree,
    version: i32,
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

fn document_symbols_method_decl(method_node: &Node, file_contents: &String) -> Vec<DocumentSymbol> {
    let mut symbols = vec![];

    if let Some(method_parameters_node) = method_node.child_by_field_name("parameters") {
        symbols.extend(document_symbols_method_params_decl(
            &method_parameters_node,
            file_contents,
        ));
    }

    symbols
}

fn document_symbols_class_decl(class_node: &Node, file_contents: &String) -> Vec<DocumentSymbol> {
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
                    document_symbols_property_decl(&cursor.node(), file_contents)
                {
                    symbols.push(prop_docsym);
                }
            } else if kind == "{" || kind == "}" || kind == "comment" {
                // ignore these
            } else if kind == "method_declaration" {
                if let Some(name_node) = cursor.node().child_by_field_name("name") {
                    let children = document_symbols_method_decl(&cursor.node(), file_contents);
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

fn document_symbols(root_node: &Node, file_contents: &String) -> Vec<DocumentSymbol> {
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
                let children = document_symbols_class_decl(&cursor.node(), file_contents);
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

struct BackendData {
    parser: Parser,

    file_trees: HashMap<Url, FileData>,
    ns_to_dir: HashMap<PhpNamespace, Vec<PathBuf>>,
}

impl BackendData {
    fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::language_php())
            .expect("error loading PHP grammar");

        Self {
            parser,
            file_trees: HashMap::new(),
            ns_to_dir: HashMap::new(),
        }
    }
}

pub struct Backend {
    client: Client,

    data: RwLock<BackendData>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,

            data: RwLock::new(BackendData::new()),
        }
    }

    async fn read_composer_file(
        &self,
        composer_file: PathBuf,
    ) -> Result<(), Box<dyn Error + Send>> {
        let file = File::open(composer_file).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        let reader = BufReader::new(file);

        let v: serde_json::Value =
            serde_json::from_reader(reader).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        if let serde_json::Value::Object(autoload) = &v["autoload"] {
            if let serde_json::Value::Object(psr4) = &autoload["psr-4"] {
                let mut data_guard = self.data.write().await;
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
                                data_guard.ns_to_dir.insert(namespace, paths);
                            }
                        }
                        serde_json::Value::String(dir) => {
                            let dir = PathBuf::from_str(dir)
                                .map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
                            data_guard.ns_to_dir.insert(namespace, vec![dir]);
                        }
                        _ => {}
                    }
                }
            }

            if let serde_json::Value::Object(psr0) = &autoload["psr-0"] {
                unimplemented!("composer autoload psr-0");
            }

            if let serde_json::Value::Array(files) = &autoload["files"] {
                unimplemented!("composer autoload files");
            }
        }

        Ok(())
    }

    async fn read_composer_files(&self, composer_files: Vec<PathBuf>) {
        for path in composer_files {
            if let Err(e) = self.read_composer_file(path).await {
                self.client.log_message(MessageType::ERROR, e).await;
            }
        }
    }
}

/**
 * Composer files paths should always exist.
 *
 * Please remember to check existence because there is a chance that it gets deleted.
 */
fn get_composer_files(workspace_folders: &Vec<WorkspaceFolder>) -> LspResult<Vec<PathBuf>> {
    let mut composer_files = vec![];
    for folder in workspace_folders {
        if let Ok(path) = folder.uri.to_file_path() {
            let composer_file = path.join("composer.json");
            if !composer_file.exists() {
                continue;
            }

            composer_files.push(composer_file);
        } else {
            continue;
        }
    }

    Ok(composer_files)
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let mut workspace_folders = params.workspace_folders.unwrap_or(vec![]);
        if workspace_folders.len() == 0 {
            if let Some(root_uri) = params.root_uri {
                workspace_folders.push(WorkspaceFolder {
                    uri: root_uri.clone(),
                    name: root_uri.to_string(),
                });
            }
        }

        if workspace_folders.len() == 0 {
            self.client
                .log_message(
                    MessageType::LOG,
                    "unable to find workspace folders, root paths, or root uris",
                )
                .await;
        } else {
            self.client
                .log_message(
                    MessageType::LOG,
                    format!(
                        "found {} workspace folders: {:?}",
                        workspace_folders.len(),
                        &workspace_folders
                    ),
                )
                .await;
        }

        // TODO check workspace folders for `composer.json` and read namespaces with PSR-4 and
        // PSR-0 (maybe support it??)
        let composer_files = get_composer_files(&workspace_folders)?;
        self.read_composer_files(composer_files).await;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        self.client
            .log_message(MessageType::LOG, "server thread has shutdown")
            .await;
        Ok(())
    }

    async fn did_open(&self, data: DidOpenTextDocumentParams) {
        let mut data_guard = self.data.write().await;
        match data_guard.parser.parse(&data.text_document.text, None) {
            Some(tree) => {
                data_guard.file_trees.insert(
                    data.text_document.uri,
                    FileData {
                        contents: data.text_document.text,
                        tree,
                        version: data.text_document.version,
                    },
                );
            }
            None => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("could not parse file `{}`", &data.text_document.uri),
                    )
                    .await
            }
        }
    }

    async fn did_change(&self, data: DidChangeTextDocumentParams) {
        // https://users.rust-lang.org/t/rwlock-is-confusing-me-and-or-mutable-borrow-counting/120492/2
        // we gently nudge the borrow checker to give us the actual &mut BackendData instead of
        // going through a DerefMut.
        let data_guard = &mut *self.data.write().await;
        match data_guard.file_trees.get_mut(&data.text_document.uri) {
            Some(entry) => {
                if entry.version >= data.text_document.version {
                    self.client
                        .log_message(
                            MessageType::LOG,
                            format!(
                                "didChange tried to change same version for file `{}`",
                                &data.text_document.uri
                            ),
                        )
                        .await;
                    return;
                }

                entry.version = data.text_document.version;
                for change in data.content_changes {
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

                    match data_guard.parser.parse(&entry.contents, None) {
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
                            &data.text_document.uri,
                        ),
                    )
                    .await;
            }
        }
    }

    async fn document_symbol(
        &self,
        data: DocumentSymbolParams,
    ) -> LspResult<Option<DocumentSymbolResponse>> {
        let data_guard = self.data.read().await;
        if let Some(FileData { contents, tree, .. }) =
            data_guard.file_trees.get(&data.text_document.uri)
        {
            Ok(Some(DocumentSymbolResponse::Nested(document_symbols(
                &tree.root_node(),
                contents,
            ))))
        } else {
            Ok(None)
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
        let actual_symbols = document_symbols(&root_node, &SOURCE.to_string());
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
