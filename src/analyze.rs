use tower_lsp::lsp_types::*;

use tree_sitter::Node;

use crate::compat::to_range;
use crate::scope::Scope;

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
        } else if kind == "arrow_function" {
            let mut arrow_function_scope = scope.clone();
            if let Some(params_node) = n.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content);
                for param in params {
                    arrow_function_scope.symbols.insert(param);
                }
            }

            if let Some(body) = n.child_by_field_name("body") {
                diagnostics.extend(walk_expression(body, content, &mut arrow_function_scope));
            }
        } else if kind == "anonymous_function" {
            let mut anonymous_scope = scope.clone();
            if let Some(params_node) = n.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content);
                for param in params {
                    anonymous_scope.symbols.insert(param);
                }
            }

            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if child.kind() == "anonymous_function_use_clause" {
                    stack.push(child);
                    break;
                }
            }

            if let Some(body) = n.child_by_field_name("body") {
                diagnostics.extend(walk_expression(body, content, &mut anonymous_scope));
            }
        } else {
            stack.extend(n.children(&mut cursor));
        }
    }

    diagnostics
}

fn walk_assignment_expression(assign: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    if let (Some(left), Some(right)) = (assign.child_by_field_name("left"), assign.child_by_field_name("right")) {
        let symbols = expression_left(left, content);
        let problems = walk_expression(right, content, scope);

        for symbol in symbols {
            scope.symbols.insert(symbol);
        }

        problems
    } else {
        Vec::new()
    }
}

fn walk_if_statement(stmt: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut cursor = stmt.walk();
    let mut diagnostics = Vec::new();
    let mut scopes = Vec::new();

    if let Some(condition) = stmt.child_by_field_name("condition") {
        let mut s = scope.clone();
        // i'm pretty sure that you can also do assignments in conditionals
        diagnostics.extend(walk_expression(condition, content, &mut s));
        scopes.push(s);
    }

    if let Some(body) = stmt.child_by_field_name("body") {
        let mut s = scope.clone();
        diagnostics.extend(walk_statement(body, content, &mut s));
        scopes.push(s);
    }

    for alt in stmt.children_by_field_name("alternative", &mut cursor) {
        let kind = alt.kind();

        if kind == "else_if_clause" {
            if let Some(condition) = alt.child_by_field_name("condition") {
                let mut s = scope.clone();
                diagnostics.extend(walk_expression(condition, content, &mut s));
                scopes.push(s);
            }
        }

        if let Some(body) = alt.child_by_field_name("body") {
            let mut s = scope.clone();
            diagnostics.extend(walk_statement(body, content, &mut s));
            scopes.push(s);
        }
    }

    for s in scopes {
        scope.absorb(s);
    }

    diagnostics
}

fn walk_class_declaration(decl: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    if let Some(name) = decl.child_by_field_name("name") {
        scope.symbols.insert(content[name.byte_range()].to_string());
    }

    if let Some(body) = decl.child_by_field_name("body") {
        if body.kind() == "declaration_list" {
            let mut cursor = body.walk();
            let mut diagnostics = Vec::new();
            for child in body.children(&mut cursor) {
                // each declaration should have it's own scope
                let mut scope = scope.clone();
                scope.symbols.insert("self".to_string());
                diagnostics.extend(walk_declaration(child, content, &mut scope));
            }

            diagnostics
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

fn walk_function_declaration(decl: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    if let Some(name) = decl.child_by_field_name("name") {
        scope.symbols.insert(content[name.byte_range()].to_string());
    }

    let mut function_scope = scope.clone();

    if let Some(params_node) = decl.child_by_field_name("parameters") {
        let params = function_parameters(params_node, content);
        for param in params {
            function_scope.symbols.insert(param);
        }
    }

    if let Some(body) = decl.child_by_field_name("body") {
        walk_statement(body, content, &mut function_scope)
    } else {
        Vec::new()
    }
}

fn walk_method_declaration(decl: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    scope.symbols.insert("$this".to_string());

    walk_function_declaration(decl, content, scope)
}

fn walk_declaration(decl: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let kind = decl.kind();

    if kind == "class_declaration" {
        walk_class_declaration(decl, content, scope)
    } else if kind == "function_declaration" {
        walk_function_declaration(decl, content, scope)
    } else if kind == "method_declaration" {
        walk_method_declaration(decl, content, scope)
    } else {
        Vec::new()
    }
}

fn walk_expression(expression: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let kind = expression.kind();

    if kind == "assignment_expression" {
        walk_assignment_expression(expression, content, scope)
    } else if kind == "parenthesized_expression" {
        if let Some(expr) = expression.child(1) {
            walk_expression(expr, content, scope)
        } else {
            expression_right(expression, content, scope)
        }
    } else {
        expression_right(expression, content, scope)
    }
}

fn walk_for_statement(statement: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut for_scope = scope.clone();

    if let Some(init) = statement.child_by_field_name("initialize") {
        diagnostics.extend(walk_expression(init, content, &mut for_scope));
    }

    if let Some(cond) = statement.child_by_field_name("condition") {
        diagnostics.extend(walk_expression(cond, content, &mut for_scope));
    }

    if let Some(update) = statement.child_by_field_name("update") {
        diagnostics.extend(walk_expression(update, content, &mut for_scope));
    }

    if let Some(body) = statement.child_by_field_name("body") {
        diagnostics.extend(walk_statement(body, content, &mut for_scope));
    }

    diagnostics
}

fn walk_foreach_statement(statement: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(iter) = statement.child(2) {
        diagnostics.extend(walk_expression(iter, content, scope));
    }

    // FIXME only references would leak
    let mut scope = scope.clone();

    if let Some(child) = statement.child(4) {
        if child.kind() == "pair" {
            let mut cursor = child.walk();
            for x in child.children(&mut cursor) {
                scope.symbols.insert(content[x.byte_range()].to_string());
            }
        } else if child.kind() == "variable_name" {
            scope.symbols.insert(content[child.byte_range()].to_string());
        }
    }

    if let Some(body) = statement.child_by_field_name("body") {
        diagnostics.extend(walk_statement(body, content, &mut scope));
    }

    diagnostics
}

fn walk_while_statement(statement: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(condition) = statement.child_by_field_name("condition") {
        diagnostics.extend(walk_expression(condition, content, scope));
    }

    if let Some(body) = statement.child_by_field_name("body") {
        diagnostics.extend(walk_statement(body, content, &mut scope.clone()));
    }

    diagnostics
}

fn walk_do_statement(statement: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let mut scope = scope.clone();
    if let Some(body) = statement.child_by_field_name("body") {
        diagnostics.extend(walk_statement(body, content, &mut scope));
    }

    if let Some(condition) = statement.child_by_field_name("condition") {
        diagnostics.extend(walk_expression(condition, content, &mut scope));
    }

    diagnostics
}

fn walk_statement(statement: Node<'_>, content: &str, scope: &mut Scope) -> Vec<Diagnostic> {
    let kind = statement.kind();

    if kind == "compound_statement" {
        let mut cursor = statement.walk();
        let mut diagnostics = Vec::new();
        for child in statement.children(&mut cursor) {
            diagnostics.extend(walk_statement(child, content, scope));
        }

        diagnostics
    } else if kind == "expression_statement" {
        if let Some(expression) = statement.child(0) {
            walk_expression(expression, content, scope)
        } else {
            Vec::new()
        }
    } else if kind == "if_statement" {
        walk_if_statement(statement, content, scope)
    } else if kind == "for_statement" {
        walk_for_statement(statement, content, scope)
    } else if kind == "foreach_statement" {
        walk_foreach_statement(statement, content, scope)
    } else if kind == "while_statement" {
        walk_while_statement(statement, content, scope)
    } else if kind == "do_statement" {
        walk_do_statement(statement, content, scope)
    } else {
        Vec::new()
    }
}

pub fn walk(node: Node<'_>, content: &str) -> Vec<Diagnostic> {
    let mut cursor = node.walk();
    let mut diagnostics = Vec::new();

    let kind = node.kind();
    if kind == "program" {
        let mut scope = Scope::empty();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "php_tag" {
                continue;
            } else if kind == "namespace_definition" {

            } else if kind == "namespace_use_declaration" {

            } else if kind.ends_with("_declaration") {
                diagnostics.extend(walk_declaration(child, content, &mut scope));
            } else if kind.ends_with("_statement") {
                diagnostics.extend(walk_statement(child, content, &mut scope));
            }
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
        let diags = super::walk_statement(stmt1, src, &mut scope);
        assert!(diags.is_empty());
        assert_eq!(1, scope.symbols.len());

        let stmt2 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt2.kind());
        let diags = super::walk_statement(stmt2, src, &mut scope);
        assert_eq!(1, diags.len());
        let diag = &diags[0];
        assert_eq!("undefined variable $var2", &diag.message);
        assert_eq!(2, scope.symbols.len());

        assert!(scope.symbols.contains("$var1"));
        assert!(scope.symbols.contains("$var2"));

        let stmt3 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt3.kind());
        let diags = super::walk_statement(stmt3, src, &mut scope);
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
            $var4 = $var3;",
            "<?php
            $container = [1, 2];
            foreach ($container as $i => $x) {
                echo $i;
                echo $x;
            }",
            "<?php
            $x = 300 + 40;
            for ($i = $x; $i < 0; $i++) {
                echo $i;
                echo $x;
            }",
            "<?php
            while ($i = 0) {
                echo $i;
            }",
            "<?php
            $f = fn($x) => $x + 1;",
            "<?php
            $b = 31;
            $f = function($x) use ($b) {return $x;};",
            "<?php
            do {
                $i = 0;
            } while ($i > 10);",
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::walk(root_node, src);
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
            "<?php
            foreach ($container as $i => $x) {
                echo $i;
                echo $x;
            }",
            "<?php
            for ($i = $x; $i < 0; $i++) {
                echo $i;
                echo $x;
            }",
            "<?php
            while ($i = $x) {
                echo $i;
            }",
            "<?php
            $f = fn($x) => $i + $x;",
            "<?php
            $f = function($x) use ($b) {return $x;};",
            "<?php
            do {
                $i = 0;
            } while ($i = $x);",
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::walk(root_node, src);
            assert!(!diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
        }
    }
}
