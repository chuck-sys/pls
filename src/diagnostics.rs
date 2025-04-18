use tower_lsp::lsp_types::*;

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};
use tree_sitter_php::language_php;

use serde::Deserialize;

use std::sync::LazyLock;

use crate::compat::to_range;
use crate::scope::Scope;

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

fn function_parameters(params: Node<'_>, content: &str) -> Vec<String> {
    let mut cursor = params.walk();
    let mut symbols = Vec::new();
    for child in params.children(&mut cursor) {
        if let Some(name_node) = child.child_by_field_name("name") {
            symbols.push(content[name_node.byte_range()].to_string());
        }
    }

    symbols
}

/// LHS of an assignment expression.
///
/// I'm not basing this off of the PHP standard, so there will be things that I get wrong.
fn expression_left(left: Node<'_>, content: &str) -> Vec<String> {
    if left.kind() == "variable_name" {
        vec![content[left.byte_range()].to_string()]
    } else if left.kind() == "list_literal" {
        let mut cursor = left.walk();
        left.children(&mut cursor)
            .into_iter()
            .filter_map(|n| (n.kind() == "variable_name").then_some(content[n.byte_range()].to_string()))
            .collect()
    } else {
        Vec::new()
    }
}

fn expression_right(right: Node<'_>, content: &str, scope: &Scope) -> Vec<Diagnostic> {
    let mut cursor = right.walk();
    let mut stack = Vec::with_capacity(10);
    let mut diagnostics = Vec::with_capacity(2);
    stack.push(right);

    while let Some(n) = stack.pop() {
        let kind = n.kind();
        if kind == "variable_name" {
            let name = &content[n.byte_range()];
            if !scope.symbols.contains(name) {
                diagnostics.push(Diagnostic {
                    range: to_range(&n.range()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: None,
                    code_description: None,
                    source: Some("undef".to_string()),
                    message: format!("undefined variable {}", name),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }

            continue;
        }

        stack.extend(n.children(&mut cursor));
    }

    diagnostics
}

fn handle_statement(stmt: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut cursor = stmt.walk();
    if !cursor.goto_first_child() {
        return vec![];
    }

    let mut stack = Vec::with_capacity(5);
    let mut diagnostics = Vec::with_capacity(2);
    stack.push(cursor.node());

    while let Some(n) = stack.pop() {
        let kind = n.kind();
        if kind == "assignment_expression" {
            if let (Some(left), Some(right)) = (n.child_by_field_name("left"), n.child_by_field_name("right")) {
                let symbols = expression_left(left, content);
                let problems = expression_right(right, content, &scope);

                diagnostics.extend(problems);

                for symbol in symbols {
                    scope.symbols.insert(symbol);
                }
            }
        } else {
            let problems = expression_right(n, content, &scope);
            diagnostics.extend(problems);
        }
    }

    diagnostics
}

pub fn undefined(node: Node<'_>, content: &str) -> Vec<Diagnostic> {
    let mut cursor = node.walk();
    let mut diagnostics = Vec::new();
    let mut stack = Vec::with_capacity(50);
    stack.push((node, Scope::empty()));

    while let Some((node, mut scope)) = stack.pop() {
        let kind = node.kind();

        if kind == "namespace_use_declaration" {
        } else if kind == "function_declaration" || kind == "method_declaration" {
            if kind == "method_declaration" {
                scope.symbols.insert("$this".to_string());
            }

            if let Some(params_node) = node.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content);
                for param in params {
                    scope.symbols.insert(param);
                }
            }
            if let Some(body_node) = node.child_by_field_name("body") {
                stack.push((body_node, scope.clone()));
            }
        } else if kind == "compound_statement" {
            for child in node.children(&mut cursor) {
                let problems = handle_statement(child, content, &mut scope);
                diagnostics.extend(problems);
            }
        } else {
            for child in node.children(&mut cursor) {
                stack.push((child, scope.clone()));
            }
        }
    }

    diagnostics
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

    use super::syntax;

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
        assert_eq!(0, syntax(tree.root_node(), SOURCE).len());
    }
}
