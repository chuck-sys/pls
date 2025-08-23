use tokio::task::JoinHandle;
use tower_lsp_server::jsonrpc::{
    Error as LspError, ErrorCode as LspErrorCode, Result as LspResult,
};
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer};

use tree_sitter::{Node, Parser};
use tree_sitter_php::language_php;
use tree_sitter_phpdoc::language as language_phpdoc;

use tokio::sync::mpsc;
use tokio::sync::{Mutex, RwLock};

use serde::Deserialize;
use serde_json::json;

use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::analyze;
use crate::code_action::{changes_phpecho, CodeActionValue, PHPECHO_TITLE};
use crate::compat::*;
use crate::composer::{get_composer_files, Autoload};
use crate::diagnostics;
use crate::diagnostics::DiagnosticsOptions;
use crate::file::{parse, FileData};
use crate::messages::AnalysisThreadMessage;
use crate::php_namespace::{PhpNamespace, SegmentPool};
use crate::stubs;
use crate::stubs::FileMapping;
use crate::types::CustomTypesDatabase;

fn document_symbols_const_decl(const_node: &Node, file_contents: &str) -> Option<DocumentSymbol> {
    let mut cursor = const_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "const_element" {
            cursor.goto_first_child();

            #[allow(deprecated)]
            return Some(DocumentSymbol {
                name: file_contents[cursor.node().byte_range()].to_string(),
                detail: Some(file_contents[const_node.byte_range()].to_string()),
                kind: SymbolKind::CONSTANT,
                tags: None,
                deprecated: None,
                range: to_range(&cursor.node().range()),
                selection_range: to_range(&const_node.range()),
                children: None,
            });
        }

        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn document_symbols_property_decl(
    property_node: &Node,
    file_contents: &str,
) -> Option<DocumentSymbol> {
    let mut cursor = property_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "property_element" {
            cursor.goto_first_child();

            #[allow(deprecated)]
            return Some(DocumentSymbol {
                name: file_contents[cursor.node().byte_range()].to_string(),
                detail: Some(file_contents[property_node.byte_range()].to_string()),
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

fn document_symbols_method_params_decl(params: &Node, file_contents: &str) -> Vec<DocumentSymbol> {
    let mut symbols = vec![];
    let mut cursor = params.walk();
    if !cursor.goto_first_child() {
        return symbols;
    }

    loop {
        let kind = cursor.node().kind();
        if kind == "simple_parameter" {
            if let Some(name_node) = cursor.node().child_by_field_name("name") {
                #[allow(deprecated)]
                symbols.push(DocumentSymbol {
                    name: file_contents[name_node.byte_range()].to_string(),
                    detail: Some(file_contents[cursor.node().byte_range()].to_string()),
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

fn document_symbols_class_decl(class_node: &Node, file_contents: &str) -> Vec<DocumentSymbol> {
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
            } else if kind == "const_declaration" {
                if let Some(const_docsym) =
                    document_symbols_const_decl(&cursor.node(), file_contents)
                {
                    symbols.push(const_docsym);
                }
            } else if kind == "{" || kind == "}" || kind == "comment" {
                // ignore these
            } else if kind == "method_declaration" {
                if let Some(name_node) = cursor.node().child_by_field_name("name") {
                    let method_name = &file_contents[name_node.byte_range()];
                    let kind = if method_name == "__constructor" {
                        SymbolKind::CONSTRUCTOR
                    } else {
                        SymbolKind::METHOD
                    };

                    #[allow(deprecated)]
                    symbols.push(DocumentSymbol {
                        name: method_name.to_string(),
                        detail: None,
                        kind,
                        tags: None,
                        deprecated: None,
                        range: to_range(&name_node.range()),
                        selection_range: to_range(&cursor.node().range()),
                        children: None,
                    });
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    symbols
}

fn document_symbols(root_node: &Node, file_contents: &str) -> Vec<DocumentSymbol> {
    let mut ret = Vec::new();
    let mut cursor = root_node.walk();

    if !cursor.goto_first_child() {
        return ret;
    }

    #[allow(deprecated)]
    loop {
        let kind = cursor.node().kind();
        // DFS
        if kind == "class_declaration" {
            if let Some(name_node) = cursor.node().child_by_field_name("name") {
                let children = document_symbols_class_decl(&cursor.node(), file_contents);
                ret.push(DocumentSymbol {
                    name: file_contents[name_node.byte_range()].to_string(),
                    detail: None,
                    kind: SymbolKind::CLASS,
                    tags: None,
                    deprecated: None,
                    range: to_range(&name_node.range()),
                    selection_range: to_range(&cursor.node().range()),
                    children: Some(children),
                });
            }
        } else if kind == "function_definition" {
            if let Some(name_node) = cursor.node().child_by_field_name("name") {
                let children = document_symbols_method_params_decl(&cursor.node(), file_contents);
                ret.push(DocumentSymbol {
                    name: file_contents[name_node.byte_range()].to_string(),
                    detail: None,
                    kind: SymbolKind::FUNCTION,
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

pub struct BackendData {
    pub php_parser: Parser,
    pub phpdoc_parser: Parser,

    pub file_trees: HashMap<Uri, FileData>,
    pub ns_store: SegmentPool,
    pub ns_to_dir: HashMap<PhpNamespace, Vec<PathBuf>>,
    pub types: CustomTypesDatabase,
}

impl BackendData {
    fn new(php_parser: Parser, phpdoc_parser: Parser) -> Self {
        Self {
            php_parser,
            phpdoc_parser,

            ns_store: SegmentPool::new(),
            file_trees: HashMap::new(),
            ns_to_dir: HashMap::new(),
            types: CustomTypesDatabase::new(),
        }
    }
}

#[derive(Deserialize, Default)]
struct InitializeOptions {
    #[serde(default)]
    diagnostics: DiagnosticsOptions,
}

pub struct Backend {
    client: Client,
    init_options: OnceLock<InitializeOptions>,
    builtins_mapping: FileMapping,

    analysis_thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    sender_to_analysis: mpsc::Sender<AnalysisThreadMessage>,

    data: Arc<RwLock<BackendData>>,
}

impl Backend {
    pub fn new<P>(stubs_filename: P, client: Client) -> Result<Self, stubs::MappingError>
    where
        P: AsRef<Path>,
    {
        let mut php_parser = Parser::new();
        php_parser
            .set_language(&language_php())
            .expect("error loading PHP grammar");

        let mut phpdoc_parser = Parser::new();
        phpdoc_parser
            .set_language(&language_phpdoc())
            .expect("error loading PHPDOC grammar");

        let builtins_mapping = FileMapping::from_filename(stubs_filename, &mut php_parser)?;
        let data = Arc::new(RwLock::new(BackendData::new(php_parser, phpdoc_parser)));
        let cloned_data = Arc::clone(&data);
        let (tx, rx) = mpsc::channel(32);
        let cloned_client = client.clone();

        let analysis_thread_handle = Arc::new(Mutex::new(Some(tokio::spawn(async move {
            analyze::main_thread(rx, cloned_data, cloned_client).await;
        }))));

        Ok(Self {
            client,
            builtins_mapping,
            init_options: OnceLock::new(),

            analysis_thread_handle,
            sender_to_analysis: tx,

            data,
        })
    }

    async fn read_composer_file(
        &self,
        composer_file: PathBuf,
    ) -> Result<(), Box<dyn Error + Send>> {
        let data_guard = &mut *self.data.write().await;

        let file = File::open(composer_file).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
        let reader = BufReader::new(file);
        let autoload = Autoload::from_reader(reader, &mut data_guard.ns_store)
            .map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;

        for (ns, dirs) in autoload.psr4.into_iter() {
            data_guard
                .ns_to_dir
                .entry(ns)
                .and_modify(|ref mut e| e.extend_from_slice(&dirs))
                .or_insert(dirs);
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

    async fn get_selection_range(&self, uri: &Uri, position: &Position) -> Option<SelectionRange> {
        let data_guard = self.data.read().await;

        if let Some(data) = data_guard.file_trees.get(uri) {
            let mut ranges = Vec::with_capacity(20);
            let root_node = data.php_tree.root_node();
            let mut node =
                root_node.named_descendant_for_point_range(to_point(position), to_point(position));

            loop {
                match node {
                    None => break,
                    Some(n) => {
                        ranges.push(SelectionRange {
                            range: to_range(&n.range()),
                            parent: None,
                        });
                        node = n.parent();
                    }
                }
            }

            if ranges.is_empty() {
                return None;
            }

            let mut parent = None;
            for mut sr in ranges.into_iter().rev() {
                sr.parent = parent;
                parent = Some(Box::new(sr));
            }

            Some(*parent.unwrap())
        } else {
            None
        }
    }

    async fn get_hover_markup(&self, uri: &Uri, position: &Position) -> Option<String> {
        let data_guard = self.data.read().await;
        let file_data = data_guard.file_trees.get(uri)?;
        let root_node = file_data.php_tree.root_node();
        let n = root_node.named_descendant_for_point_range(to_point(position), to_point(position))?;
        if n.kind() == "name" {
            let parent = n.parent()?;
            match parent.kind() {
                "variable_name" => Some("Variable!".to_string()),
                "class_declaration" => {
                    None
                }
                _ => None,
            }
        } else {
            Some(n.to_string())
        }
    }
}

fn supported_capabilities() -> &'static ServerCapabilities {
    static CAPS: OnceLock<ServerCapabilities> = OnceLock::new();
    CAPS.get_or_init(|| ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_symbol_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::SOURCE]),
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: Some(false),
            },
            resolve_provider: Some(true),
        })),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    })
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let mut workspace_folders = params.workspace_folders.unwrap_or(vec![]);
        if workspace_folders.is_empty() {
            #[allow(deprecated)]
            if let Some(root_uri) = params.root_uri {
                workspace_folders.push(WorkspaceFolder {
                    uri: root_uri.clone(),
                    name: root_uri.to_string(),
                });
            }
        }

        if workspace_folders.is_empty() {
            self.client
                .show_message(
                    MessageType::LOG,
                    "unable to find workspace folders, root paths, or root uris",
                )
                .await;
        } else {
            self.client
                .show_message(
                    MessageType::LOG,
                    format!("found {} workspace folder(s)", workspace_folders.len()),
                )
                .await;
        }

        let composer_files = get_composer_files(&workspace_folders);
        self.read_composer_files(composer_files).await;

        if let Some(v) = params.initialization_options {
            match serde_json::from_value(v) {
                Ok(v) => {
                    self.init_options.get_or_init(|| v);
                }
                Err(e) => {
                    self.init_options
                        .get_or_init(|| InitializeOptions::default());
                    return Err(LspError {
                        code: LspErrorCode::InvalidParams,
                        message: Cow::Borrowed("bad init options"),
                        data: Some(e.to_string().into()),
                    });
                }
            }
        }

        Ok(InitializeResult {
            capabilities: supported_capabilities().clone(),
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn shutdown(&self) -> LspResult<()> {
        self.client
            .log_message(MessageType::LOG, "server shutdown")
            .await;

        if let Err(e) = self
            .sender_to_analysis
            .send(AnalysisThreadMessage::Shutdown)
            .await
        {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("analysis thread already shutdown with message: {}", e),
                )
                .await;
        }
        let mut mtx = self.analysis_thread_handle.lock().await;
        if let Some(handle) = mtx.take() {
            let _ = handle.await;
        }

        Ok(())
    }

    async fn did_open(&self, data: DidOpenTextDocumentParams) {
        let data_guard = &mut *self.data.write().await;
        let (php_tree, comments_tree) = parse(
            (&mut data_guard.php_parser, &mut data_guard.phpdoc_parser),
            &data.text_document.text,
            (None, None),
        );

        let mut diagnostics = vec![];
        if self.init_options.get().unwrap().diagnostics.syntax {
            diagnostics.extend(diagnostics::syntax(
                php_tree.root_node(),
                &data.text_document.text,
            ));
        }

        diagnostics.extend(analyze::walk(
            php_tree.root_node(),
            &data.text_document.text,
            &mut data_guard.ns_store,
        ));

        let _ = self.sender_to_analysis.send(AnalysisThreadMessage::AnalyzeUri(data.text_document.uri.clone())).await;

        self.client
            .publish_diagnostics(
                data.text_document.uri.clone(),
                diagnostics,
                Some(data.text_document.version),
            )
            .await;

        data_guard.file_trees.insert(
            data.text_document.uri,
            FileData {
                php_tree,
                comments_tree,
                contents: data.text_document.text,
                version: data.text_document.version,
            },
        );
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
                            MessageType::WARNING,
                            format!(
                                "didChange tried to change same version for file `{}`",
                                data.text_document.uri.as_str(),
                            ),
                        )
                        .await;
                    return;
                }

                entry.version = data.text_document.version;
                for c in data.content_changes {
                    match entry.change(c) {
                        Err(e) => self.client.log_message(MessageType::ERROR, e).await,
                        _ => {}
                    }
                }

                let (php_tree, comments_tree) = parse(
                    (&mut data_guard.php_parser, &mut data_guard.phpdoc_parser),
                    &entry.contents,
                    (Some(&entry.php_tree), Some(&entry.comments_tree)),
                );

                entry.php_tree = php_tree;
                entry.comments_tree = comments_tree;

                let mut diagnostics = vec![];
                if self.init_options.get().unwrap().diagnostics.syntax {
                    diagnostics.extend(diagnostics::syntax(
                        entry.php_tree.root_node(),
                        &entry.contents,
                    ));
                }

                diagnostics.extend(analyze::walk(
                    entry.php_tree.root_node(),
                    &entry.contents,
                    &mut data_guard.ns_store,
                ));

                self.client
                    .publish_diagnostics(
                        data.text_document.uri.clone(),
                        diagnostics,
                        Some(data.text_document.version),
                    )
                    .await;
            }
            None => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!(
                            "didChange event triggered without didOpen for file `{}`",
                            data.text_document.uri.as_str(),
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
        if let Some(FileData {
            contents, php_tree, ..
        }) = data_guard.file_trees.get(&data.text_document.uri)
        {
            Ok(Some(DocumentSymbolResponse::Nested(document_symbols(
                &php_tree.root_node(),
                contents,
            ))))
        } else {
            self.client
                .log_message(
                    MessageType::ERROR,
                    "documentSymbol could not find any file of this uri",
                )
                .await;
            Ok(None)
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        let mut responses = vec![];
        let data_guard = self.data.read().await;
        if let Some(file_data) = data_guard.file_trees.get(&params.text_document.uri) {
            if params.range.start == params.range.end && file_data.contents.contains("<?php echo ")
            {
                let action = CodeAction {
                    title: PHPECHO_TITLE.to_string(),
                    kind: Some(CodeActionKind::SOURCE),
                    data: Some(json!({"uri": params.text_document.uri})),
                    ..CodeAction::default()
                };
                responses.push(CodeActionOrCommand::CodeAction(action));
            }
        }
        Ok(Some(responses))
    }

    async fn code_action_resolve(&self, params: CodeAction) -> LspResult<CodeAction> {
        if &params.title == PHPECHO_TITLE {
            if let Some(v) = params.data {
                let v: CodeActionValue = serde_json::from_value(v).map_err(|e| LspError {
                    code: LspErrorCode::InvalidParams,
                    message: Cow::Borrowed("malformed code action data"),
                    data: Some(e.to_string().into()),
                })?;
                let data_guard = self.data.read().await;
                let file_data = data_guard.file_trees.get(&v.uri).ok_or(LspError {
                    code: LspErrorCode::InternalError,
                    message: Cow::Borrowed("could not find file data"),
                    data: Some(v.uri.to_string().into()),
                })?;

                let document_changes =
                    changes_phpecho(&v.uri, &file_data.contents, file_data.version);

                Ok(CodeAction {
                    title: PHPECHO_TITLE.to_string(),
                    kind: Some(CodeActionKind::SOURCE),
                    edit: Some(WorkspaceEdit {
                        document_changes,
                        ..WorkspaceEdit::default()
                    }),
                    ..CodeAction::default()
                })
            } else {
                Err(LspError {
                    code: LspErrorCode::InvalidRequest,
                    message: Cow::Borrowed("missing params data from code action"),
                    data: None,
                })
            }
        } else {
            Err(LspError {
                code: LspErrorCode::InvalidRequest,
                message: Cow::Borrowed("unsupported code action resolve request"),
                data: Some(params.title.into()),
            })
        }
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> LspResult<Option<Vec<SelectionRange>>> {
        let mut acc = Vec::with_capacity(params.positions.len());

        for position in params.positions {
            if let Some(sr) = self
                .get_selection_range(&params.text_document.uri, &position)
                .await
            {
                acc.push(sr);
            }
        }

        Ok(Some(acc))
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = &params.text_document_position_params.position;

        if let Some(content) = self.get_hover_markup(uri, position).await {
            Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            }))
        } else {
            Ok(None)
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        Ok(Some(GotoDefinitionResponse::Link(Vec::new())))
    }
}

#[cfg(test)]
mod test {
    use tower_lsp_server::lsp_types::*;
    use tree_sitter::Parser;
    use tree_sitter_php::language_php;

    use super::document_symbols;

    fn parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&language_php())
            .expect("error loading PHP grammar");

        parser
    }

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
    fn get_symbols() {
        let tree = parser().parse(SOURCE, None).unwrap();
        let root_node = tree.root_node();
        let actual_symbols = document_symbols(&root_node, &SOURCE.to_string());
        assert_eq!(2, actual_symbols.len());
        assert_eq!("Whatever", &actual_symbols[0].name);
        assert_eq!("Another", &actual_symbols[1].name);
        assert_eq!(3, actual_symbols[0].children.as_ref().unwrap().len());
        assert_eq!("$x", &actual_symbols[0].children.as_ref().unwrap()[0].name);
        assert_eq!("foo", &actual_symbols[0].children.as_ref().unwrap()[1].name);
        assert_eq!("fee", &actual_symbols[0].children.as_ref().unwrap()[2].name);
        assert!(actual_symbols[0].children.as_ref().unwrap()[0]
            .children
            .is_none());
        assert!(actual_symbols[0].children.as_ref().unwrap()[2]
            .children
            .is_none());
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
