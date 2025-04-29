use tower_lsp::lsp_types::*;

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};
use tree_sitter_php::language_php;

use serde::Deserialize;

use std::sync::LazyLock;

use crate::compat::to_range;

static MISSING_QUERY: LazyLock<Query> = LazyLock::new(|| Query::new(&language_php(), "(MISSING) @missings").unwrap());
static ERROR_QUERY: LazyLock<Query> = LazyLock::new(|| Query::new(&language_php(), "(ERROR) @error").unwrap());

#[derive(Deserialize)]
pub struct DiagnosticsOptions {
    #[serde(default)]
    pub syntax: bool,

    #[serde(default)]
    pub undefined: bool,
}

impl Default for DiagnosticsOptions {
    fn default() -> Self {
        Self {
            syntax: true,
            undefined: true,
        }
    }
}

pub fn syntax(node: Node<'_>, content: &str) -> Vec<Diagnostic> {
    let mut missings = get_tree_diagnostics_missing(node, content);
    let errors = get_tree_diagnostics_errors(node, content);

    missings.extend(errors);

    missings
}

fn get_tree_diagnostics_missing(node: Node<'_>, content: &str) -> Vec<Diagnostic> {
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&MISSING_QUERY, node, content.as_bytes());
    let mut diagnostics = Vec::new();

    while let Some((m, _)) = captures.next() {
        for c in m.captures.iter() {
            let sexp = c.node.to_sexp();
            diagnostics.push(Diagnostic {
                range: to_range(&c.node.range()),
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("ts".to_string()),
                message: sexp[1..sexp.len() - 1].to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }

    diagnostics
}

fn get_tree_diagnostics_errors(node: Node<'_>, content: &str) -> Vec<Diagnostic> {
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&ERROR_QUERY, node, content.as_bytes());
    let mut diagnostics = Vec::new();

    while let Some((m, _)) = captures.next() {
        for c in m.captures.iter() {
            diagnostics.push(Diagnostic {
                range: to_range(&c.node.range()),
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("ts".to_string()),
                message: format!("UNEXPECTED: {}", &content[c.node.byte_range()]),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }

    diagnostics
}

#[cfg(test)]
mod test {
    use tree_sitter::Parser;
    use tree_sitter_php::language_php;

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
    fn no_diags() {
        let tree = parser().parse(SOURCE, None).unwrap();
        assert_eq!(0, super::syntax(tree.root_node(), SOURCE).len());
    }
}
