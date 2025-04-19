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

    let mut diagnostics = Vec::with_capacity(2);

    let kind = stmt.kind();
    if kind == "if_statement" {
        return handle_branch(stmt, content, scope);
    }

    for child in stmt.children(&mut cursor) {
        let kind = child.kind();

        if kind == "assignment_expression" {
            if let (Some(left), Some(right)) = (child.child_by_field_name("left"), child.child_by_field_name("right")) {
                let symbols = expression_left(left, content);
                let problems = expression_right(right, content, &scope);

                diagnostics.extend(problems);

                for symbol in symbols {
                    scope.symbols.insert(symbol);
                }
            }
        } else {
            diagnostics.extend(expression_right(child, content, &scope));
        }
    }

    diagnostics
}

fn handle_branch(stmt: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut cursor = stmt.walk();
    let mut diagnostics = Vec::new();
    let mut scopes = Vec::new();

    if let Some(condition) = stmt.child_by_field_name("condition") {
        let mut s = scope.clone();
        // i'm pretty sure that you can also do assignments in conditionals
        diagnostics.extend(handle_statement(condition, content, &mut s));
        scopes.push(s);
    }

    if let Some(body) = stmt.child_by_field_name("body") {
        let mut s = scope.clone();
        for child in body.children(&mut cursor) {
            diagnostics.extend(handle_statement(child, content, &mut s));
        }
        scopes.push(s);
    }

    for alt in stmt.children_by_field_name("alternative", &mut cursor) {
        let kind = alt.kind();

        if kind == "else_if_clause" {
            if let Some(condition) = alt.child_by_field_name("condition") {
                let mut s = scope.clone();
                diagnostics.extend(handle_statement(condition, content, &mut s));
                scopes.push(s);
            }
        }

        if let Some(body) = alt.child_by_field_name("body") {
            let mut s = scope.clone();
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                diagnostics.extend(handle_statement(child, content, &mut s));
            }
            scopes.push(s);
        }
    }

    for s in scopes {
        scope.absorb(s);
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
                scope.symbols.insert("self".to_string());
            }

            if let Some(params_node) = node.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content);
                for param in params {
                    scope.symbols.insert(param);
                }
            }
            if let Some(body_node) = node.child_by_field_name("body") {
                stack.push((body_node, scope));
            }
        } else if kind == "compound_statement" {
            for child in node.children(&mut cursor) {
                diagnostics.extend(handle_statement(child, content, &mut scope));
            }
        } else if kind == "php_tag" {
            // ignore
        } else if kind == "for_statement" {
            if let Some(init) = node.child_by_field_name("initialize") {
                diagnostics.extend(handle_statement(init, content, &mut scope));
            }

            if let Some(cond) = node.child_by_field_name("condition") {
                diagnostics.extend(handle_statement(cond, content, &mut scope));
            }

            if let Some(body) = node.child_by_field_name("body") {
                stack.push((body, scope));
            }
        } else if kind == "foreach_statement" {
            if let Some(iter) = node.child(0) {
                diagnostics.extend(expression_right(iter, content, &mut scope));
            }

            if let Some(child) = node.child(1) {
                if child.kind() == "pair" {
                    for x in child.children(&mut cursor) {
                        scope.symbols.insert(content[x.byte_range()].to_string());
                    }
                } else if child.kind() == "variable_name" {
                    scope.symbols.insert(content[child.byte_range()].to_string());
                }
            }

            if let Some(body) = node.child_by_field_name("body") {
                stack.push((body, scope));
            }
        } else {
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                if kind == "expression_statement" {
                    diagnostics.extend(handle_statement(child, content, &mut scope));
                } else if kind == "if_statement" {
                    diagnostics.extend(handle_branch(child, content, &mut scope));
                } else {
                    stack.push((child, scope.clone()));
                }
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

    use crate::scope::Scope;

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

    #[test]
    fn assignments_scoping() {
        let src = "<?php
        $var1 = 1 + 2;
        $var2 = $var1 + $var2;
        list($var3, $var4) = [$var1, $var4 + 2];
        ";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let mut cursor = root_node.walk();
        let mut scope = Scope::empty();
        let mut iter = root_node.children(&mut cursor);

        // skip `<?php` tag
        iter.next();

        let stmt1 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt1.kind());
        let diags = super::handle_statement(stmt1, src, &mut scope);
        assert!(diags.is_empty());
        assert_eq!(1, scope.symbols.len());

        let stmt2 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt2.kind());
        let diags = super::handle_statement(stmt2, src, &mut scope);
        assert_eq!(1, diags.len());
        let diag = &diags[0];
        assert_eq!("undefined variable $var2", &diag.message);
        assert_eq!(2, scope.symbols.len());

        assert!(scope.symbols.contains("$var1"));
        assert!(scope.symbols.contains("$var2"));

        let stmt3 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt3.kind());
        let diags = super::handle_statement(stmt3, src, &mut scope);
        assert_eq!(1, diags.len());
        let diag = &diags[0];
        assert_eq!("undefined variable $var4", &diag.message);
        assert_eq!(4, scope.symbols.len());

        assert!(scope.symbols.contains("$var3"));
        assert!(scope.symbols.contains("$var4"));
    }

    #[test]
    fn no_undefineds() {
        let srcs = [
            "<?php
            $var1 = 1 + 2;
            $var2 = $var1 + 3;",
            "<?php
            $var1 = 1 + 2;
            class Foo {
                private function x(): void {
                    $var2 = $var1 + 2;
                }
            }",
            "<?php
            $var1 = 1;
            if ($var1 === 2) {
                $var2 = 3;
                if ($var2 === 3) {}
            } else {
                $var3 = 4;
            }
            $var4 = $var3;"
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::undefined(root_node, src);
            assert!(diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
        }
    }

    #[test]
    fn non_zero_undefineds() {
        let srcs = [
            "<?php
            $var1 = 1 + 2;
            $var2 = $var1 + $var2;",
            "<?php
            $var1 = 2;
            if ($var2 == 5) {}",
            "<?php
            if (true) {
                $var1 = 4;
            } else {
                $var2 = $var1;
            }",
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::undefined(root_node, src);
            assert!(!diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
        }
    }
}
