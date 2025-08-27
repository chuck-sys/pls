use tower_lsp_server::{lsp_types::*, Client, UriExt};

use tree_sitter::Node;

use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;

use std::collections::VecDeque;
use std::sync::Arc;

use crate::backend::BackendData;
use crate::compat::to_range;
use crate::messages::{AnalysisThreadMessage, AnalysisThreadQueueItem};
use crate::php_namespace::{resolve_ns, PhpNamespace, SegmentPool};
use crate::scope::{Scope, SUPERGLOBALS};
use crate::types::{
    Class, CustomType, CustomTypeMeta, CustomTypesDatabase, FromNode, Method, Property, Type,
    Visibility,
};

pub async fn main_thread(
    mut rx: Receiver<AnalysisThreadMessage>,
    data: Arc<RwLock<BackendData>>,
    client: Client,
) {
    let mut q = VecDeque::new();

    /// Max number of items from queue to run per `recv`
    const PROCESS_ITEMS_PER_RECV: usize = 10;

    while let Some(msg) = rx.recv().await {
        use AnalysisThreadMessage::*;

        match msg {
            Shutdown => break,
            AnalyzeUri(uri) => q.push_back(AnalysisThreadQueueItem::Uri(uri)),
            AnalyzeNs(ns) => q.push_back(AnalysisThreadQueueItem::Ns(ns)),
        }

        for _ in 0..PROCESS_ITEMS_PER_RECV {
            let data_lock = &mut *data.write().await;
            match q.pop_back() {
                Some(AnalysisThreadQueueItem::Uri(uri)) => {
                    let dependencies = if let Some(filedata) = data_lock.file_trees.get(&uri) {
                        injest_types(
                            filedata.php_tree.root_node(),
                            &filedata.contents,
                            &mut data_lock.ns_store,
                            &mut data_lock.types,
                        )
                    } else {
                        todo!("they should be processed, idk why they aren't");
                    };

                    for dep_ns in dependencies.into_iter() {
                        q.push_back(AnalysisThreadQueueItem::Ns(dep_ns));
                    }
                }
                Some(AnalysisThreadQueueItem::Ns(mut ns)) => {
                    match ns.pop() {
                        Some(base) => {
                            match resolve_ns(&ns, &data_lock.ns_to_dir) {
                                Ok(dir) => {
                                    let path = dir.join(format!("{base}.php"));
                                    match std::fs::read_to_string(path) {
                                        Ok(contents) => {
                                            let php_tree = data_lock.php_parser.parse(&contents, None).unwrap();
                                            let dependencies = injest_types(
                                                php_tree.root_node(),
                                                &contents,
                                                &mut data_lock.ns_store,
                                                &mut data_lock.types,
                                            );
                                            for dep_ns in dependencies.into_iter() {
                                                q.push_back(AnalysisThreadQueueItem::Ns(dep_ns));
                                            }
                                        }
                                        Err(e) => client.log_message(MessageType::ERROR, e.to_string()).await,
                                    }
                                },
                                Err(e) => client.log_message(MessageType::ERROR, e.to_string()).await,
                            }
                        },
                        None => {},
                    }
                }
                _ => break,
            }
        }
    }
}

fn function_parameters(
    params: Node<'_>,
    content: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<String> {
    let mut cursor = params.walk();
    let mut symbols = Vec::new();

    for child in params.children(&mut cursor) {
        if let Some(name_node) = child.child_by_field_name("name") {
            let name = &content[name_node.byte_range()];

            symbols.push(name.to_string());

            if SUPERGLOBALS.contains(name) {
                diagnostics.push(Diagnostic {
                    range: to_range(&name_node.range()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("superglobal".to_string()),
                    message: format!("superglobal {} cannot be shadowed", name),
                    ..Default::default()
                });
            }
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
            .filter_map(|n| {
                (n.kind() == "variable_name").then_some(content[n.byte_range()].to_string())
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn expression_right(
    right: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cursor = right.walk();
    let mut stack = Vec::with_capacity(10);
    stack.push(right);

    while let Some(n) = stack.pop() {
        let kind = n.kind();
        if kind == "variable_name" {
            let name = &content[n.byte_range()];
            if !scope.symbols.contains(name) {
                diagnostics.push(Diagnostic {
                    range: to_range(&n.range()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("undef".to_string()),
                    message: format!("undefined variable {}", name),
                    ..Default::default()
                });
            }
        } else if kind == "arrow_function" {
            let mut arrow_function_scope = scope.clone();
            if let Some(params_node) = n.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content, diagnostics);
                for param in params {
                    arrow_function_scope.symbols.insert(param);
                }
            }

            if let Some(body) = n.child_by_field_name("body") {
                walk_expression(
                    body,
                    content,
                    ns_store,
                    &mut arrow_function_scope,
                    diagnostics,
                );
            }
        } else if kind == "anonymous_function" {
            let mut anonymous_scope = scope.clone();
            if let Some(params_node) = n.child_by_field_name("parameters") {
                let params = function_parameters(params_node, content, diagnostics);
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
                walk_statement(body, content, ns_store, &mut anonymous_scope, diagnostics);
            }
        } else {
            stack.extend(n.children(&mut cursor));
        }
    }
}

fn walk_assignment_expression(
    assign: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let (Some(left), Some(right)) = (
        assign.child_by_field_name("left"),
        assign.child_by_field_name("right"),
    ) {
        let symbols = expression_left(left, content);
        walk_expression(right, content, ns_store, scope, diagnostics);

        for symbol in symbols {
            scope.symbols.insert(symbol);
        }
    }
}

fn walk_if_statement(
    stmt: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cursor = stmt.walk();
    let mut scopes = Vec::new();

    if let Some(condition) = stmt.child_by_field_name("condition") {
        let mut s = scope.clone();
        // i'm pretty sure that you can also do assignments in conditionals
        walk_expression(condition, content, ns_store, &mut s, diagnostics);
        scopes.push(s);
    }

    if let Some(body) = stmt.child_by_field_name("body") {
        let mut s = scope.clone();
        walk_statement(body, content, ns_store, &mut s, diagnostics);
        scopes.push(s);
    }

    for alt in stmt.children_by_field_name("alternative", &mut cursor) {
        let kind = alt.kind();

        if kind == "else_if_clause" {
            if let Some(condition) = alt.child_by_field_name("condition") {
                let mut s = scope.clone();
                walk_expression(condition, content, ns_store, &mut s, diagnostics);
                scopes.push(s);
            }
        }

        if let Some(body) = alt.child_by_field_name("body") {
            let mut s = scope.clone();
            walk_statement(body, content, ns_store, &mut s, diagnostics);
            scopes.push(s);
        }
    }

    for s in scopes {
        scope.absorb(s);
    }
}

fn walk_class_declaration(
    decl: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut t = Class::default();
    let mut markup = None;

    if let Some(prev) = decl.prev_sibling() {
        if prev.kind() == "comment" {
            let comment = &content[prev.byte_range()];
            if comment.starts_with("/**") {
                markup = Some(comment.to_string());
            }
        }
    }

    if let Some(name) = decl.child_by_field_name("name") {
        scope.symbols.insert(content[name.byte_range()].to_string());
        t.name = content[name.byte_range()].to_string();
    }

    if let Some(body) = decl.child_by_field_name("body") {
        if body.kind() == "declaration_list" {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                // each declaration should have it's own scope
                let mut scope = scope.clone();
                scope.symbols.insert("self".to_string());
                walk_declaration(child, content, ns_store, &mut scope, diagnostics);
            }
        }
    }
}

fn walk_function_declaration(
    decl: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(name) = decl.child_by_field_name("name") {
        scope.symbols.insert(content[name.byte_range()].to_string());
    }

    let mut function_scope = scope.clone();

    if let Some(params_node) = decl.child_by_field_name("parameters") {
        let params = function_parameters(params_node, content, diagnostics);
        for param in params {
            function_scope.symbols.insert(param);
        }
    }

    if let Some(body) = decl.child_by_field_name("body") {
        walk_statement(body, content, ns_store, &mut function_scope, diagnostics);
    }
}

fn walk_method_declaration(
    decl: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    scope.symbols.insert("$this".to_string());

    walk_function_declaration(decl, content, ns_store, scope, diagnostics)
}

fn walk_declaration(
    decl: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let kind = decl.kind();

    if kind == "class_declaration" {
        walk_class_declaration(decl, content, ns_store, scope, diagnostics)
    } else if kind == "function_definition" || kind == "function_static_declaration" {
        walk_function_declaration(decl, content, ns_store, scope, diagnostics)
    } else if kind == "method_declaration" {
        walk_method_declaration(decl, content, ns_store, scope, diagnostics)
    }
}

fn walk_expression(
    expression: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let kind = expression.kind();

    if kind.ends_with("assignment_expression") {
        walk_assignment_expression(expression, content, ns_store, scope, diagnostics)
    } else if kind == "parenthesized_expression" {
        if let Some(expr) = expression.child(1) {
            walk_expression(expr, content, ns_store, scope, diagnostics)
        } else {
            expression_right(expression, content, ns_store, scope, diagnostics)
        }
    } else {
        expression_right(expression, content, ns_store, scope, diagnostics)
    }
}

fn walk_for_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(init) = statement.child_by_field_name("initialize") {
        walk_expression(init, content, ns_store, scope, diagnostics);
    }

    if let Some(cond) = statement.child_by_field_name("condition") {
        walk_expression(cond, content, ns_store, scope, diagnostics);
    }

    if let Some(update) = statement.child_by_field_name("update") {
        walk_expression(update, content, ns_store, scope, diagnostics);
    }

    if let Some(body) = statement.child_by_field_name("body") {
        walk_statement(body, content, ns_store, scope, diagnostics);
    }
}

fn walk_foreach_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(iter) = statement.child(2) {
        walk_expression(iter, content, ns_store, scope, diagnostics);
    }

    if let Some(child) = statement.child(4) {
        if child.kind() == "pair" {
            let mut cursor = child.walk();
            for x in child.children(&mut cursor) {
                scope.symbols.insert(content[x.byte_range()].to_string());
            }
        } else if child.kind() == "variable_name" {
            scope
                .symbols
                .insert(content[child.byte_range()].to_string());
        } else if child.kind() == "by_ref" {
            if let Some(v) = child.child(1) {
                scope.symbols.insert(content[v.byte_range()].to_string());
            }
        }
    }

    if let Some(body) = statement.child_by_field_name("body") {
        walk_statement(body, content, ns_store, scope, diagnostics);
    }
}

fn walk_while_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(condition) = statement.child_by_field_name("condition") {
        walk_expression(condition, content, ns_store, scope, diagnostics);
    }

    if let Some(body) = statement.child_by_field_name("body") {
        walk_statement(body, content, ns_store, scope, diagnostics);
    }
}

fn walk_do_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(body) = statement.child_by_field_name("body") {
        walk_statement(body, content, ns_store, scope, diagnostics);
    }

    if let Some(condition) = statement.child_by_field_name("condition") {
        walk_expression(condition, content, ns_store, scope, diagnostics);
    }
}

fn walk_switch_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(expr) = statement.child_by_field_name("condition") {
        walk_expression(expr, content, ns_store, scope, diagnostics);
    }

    if let Some(body) = statement.child_by_field_name("body") {
        let mut cursor = body.walk();
        for statement in body.children(&mut cursor) {
            if statement.kind() == "case_statement" || statement.kind() == "default_statement" {
                if let Some(name) = statement.child_by_field_name("value") {
                    walk_expression(name, content, ns_store, scope, diagnostics);
                }

                let mut another_cursor = statement.walk();
                for s in statement.children(&mut another_cursor) {
                    walk_statement(s, content, ns_store, scope, diagnostics);
                }
            }
        }
    }
}

fn walk_statement(
    statement: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let kind = statement.kind();

    if kind == "compound_statement" {
        let mut cursor = statement.walk();
        for child in statement.children(&mut cursor) {
            walk_statement(child, content, ns_store, scope, diagnostics);
        }
    } else if kind == "expression_statement" {
        if let Some(expression) = statement.child(0) {
            walk_expression(expression, content, ns_store, scope, diagnostics);
        }
    } else if kind == "if_statement" {
        walk_if_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "for_statement" {
        walk_for_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "foreach_statement" {
        walk_foreach_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "while_statement" {
        walk_while_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "do_statement" {
        walk_do_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "switch_statement" {
        walk_switch_statement(statement, content, ns_store, scope, diagnostics);
    } else if kind == "echo_statement" {
        let mut cursor = statement.walk();
        for child in statement.children(&mut cursor) {
            walk_expression(child, content, ns_store, scope, diagnostics);
        }
    }
}

pub fn walk_ns_use_clause(
    node: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut ns = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            ns = Some(ns_store.intern_str(&content[child.byte_range()]));
            break;
        }
    }

    if let Some(ns) = ns {
        if let Some(alias) = node.child_by_field_name("alias") {
            if scope.ns_aliases.contains_key(&content[alias.byte_range()]) {
                diagnostics.push(Diagnostic {
                    range: to_range(&node.range()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("dupe".to_string()),
                    message: format!(
                        "namespace alias {} already declared",
                        &content[alias.byte_range()]
                    ),
                    ..Default::default()
                });
            } else {
                scope
                    .ns_aliases
                    .insert(content[alias.byte_range()].to_string(), ns);
            }
        } else {
            let alias = ns.0[ns.len() - 1].to_string();
            if scope.ns_aliases.contains_key(&alias) {
                diagnostics.push(Diagnostic {
                    range: to_range(&node.range()),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("dupe".to_string()),
                    message: format!("namespace alias {} already declared", &alias),
                    ..Default::default()
                });
            } else {
                scope.ns_aliases.insert(alias, ns);
            }
        }
    }
}

pub fn walk_ns_use_declaration(
    node: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    scope: &mut Scope,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "namespace_use_clause" {
            walk_ns_use_clause(child, content, ns_store, scope, diagnostics);
        }
    }
}

pub fn walk(node: Node<'_>, content: &str, ns_store: &mut SegmentPool) -> Vec<Diagnostic> {
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
                if let Some(name) = child.child_by_field_name("name") {
                    let ns = ns_store.intern_str(&content[name.byte_range()]);
                    scope.ns = Some(ns);
                }
            } else if kind == "namespace_use_declaration" {
                walk_ns_use_declaration(child, content, ns_store, &mut scope, &mut diagnostics);
            } else if kind.ends_with("_declaration") || kind == "function_definition" {
                walk_declaration(child, content, ns_store, &mut scope, &mut diagnostics);
            } else if kind.ends_with("_statement") {
                walk_statement(child, content, ns_store, &mut scope, &mut diagnostics);
            }
        }
    }

    diagnostics
}

/// Fills out types database.
///
/// We fill out the types database in this pass. We don't check for any kinds of errors; that'll be
/// after we fill out the types database.
///
/// We obtain a list of type dependencies. These should be resolved by the caller.
pub fn injest_types(
    node: Node<'_>,
    content: &str,
    ns_store: &mut SegmentPool,
    types: &mut CustomTypesDatabase,
) -> Vec<PhpNamespace> {
    let mut cursor = node.walk();
    let mut dependencies = Vec::new();

    let kind = node.kind();
    if kind == "program" {
        let mut scope = Scope::empty();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "php_tag" {
                continue;
            } else if kind == "namespace_definition" {
                if let Some(name) = child.child_by_field_name("name") {
                    let ns = ns_store.intern_str(&content[name.byte_range()]);
                    scope.ns = Some(ns);
                }
            } else if kind == "namespace_use_declaration" {
                // XXX create new fn for mutating scope without diagnostics
                walk_ns_use_declaration(child, content, ns_store, &mut scope, &mut Vec::new());
            } else if kind == "class_declaration" {
                injest_class_declaration(
                    child,
                    content,
                    &scope,
                    ns_store,
                    types,
                    &mut dependencies,
                );
            } else if kind.ends_with("_declaration") || kind == "function_definition" {
                // walk_declaration(
                //     child,
                //     content,
                //     ns_store,
                //     &mut scope,
                //     types,
                //     &mut diagnostics,
                // );
            } else if kind.ends_with("_statement") {
                // walk_statement(child, content, ns_store, &mut scope, &mut diagnostics);
            }
        }
    }

    dependencies
}

fn node_markup(node: Node<'_>, content: &str) -> Option<String> {
    if let Some(prev) = node.prev_sibling() {
        if prev.kind() == "comment" {
            let comment = &content[prev.byte_range()];
            if comment.starts_with("/**") {
                return Some(comment.to_string());
            }
        }
    }

    None
}

/// Get all children that have `node.kind() == "name"`.
///
/// Return a list of FQN.
pub fn clause_fqn_names(
    node: Node<'_>,
    content: &str,
    scope: &Scope,
    ns_store: &mut SegmentPool,
) -> Vec<PhpNamespace> {
    let mut cursor = node.walk();
    let mut names = Vec::new();

    for child in node.children(&mut cursor) {
        if !child.kind().ends_with("name") {
            continue;
        }

        let name = &content[child.byte_range()];
        if child.kind() == "name" {
            if let Some(ns) = scope.ns_aliases.get(name) {
                names.push(ns.clone());
            } else {
                let mut ns = scope.ns.clone().unwrap_or(PhpNamespace::empty());
                ns.0.push(Arc::from(name));
                names.push(ns);
            }
        } else if child.kind() == "qualified_name" {
            if name.starts_with("\\") {
                names.push(ns_store.intern_str(name));
            } else {
                let relative_ns = ns_store.intern_str(name);
                if let Some(first_segment) = relative_ns.0.get(0) {
                    if let Some(ns) = scope.ns_aliases.get(first_segment.as_ref()) {
                        let mut ns = ns.clone();
                        ns.pop();
                        ns.extend(relative_ns.0.into_iter());
                        names.push(ns);
                    } else {
                        let mut ns = scope.ns.clone().unwrap_or(PhpNamespace::empty());
                        ns.extend(relative_ns.0.into_iter());
                        names.push(ns);
                    }
                }
            }
        }
    }

    names
}

pub fn injest_class_declaration(
    node: Node<'_>,
    content: &str,
    scope: &Scope,
    ns_store: &mut SegmentPool,
    types: &mut CustomTypesDatabase,
    dependencies: &mut Vec<PhpNamespace>,
) {
    let mut t = Class::default();
    let markup = node_markup(node, content);

    if let Some(name) = node.child_by_field_name("name") {
        t.name = content[name.byte_range()].to_string();
    }

    if let Some(body) = node.child_by_field_name("body") {
        if body.kind() == "declaration_list" {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "property_declaration" {
                    if let Ok(property) = Property::from_node(child, content) {
                        t.properties.insert(property.name.clone(), property);
                    }
                } else if child.kind() == "method_declaration" {
                    if let Ok(method) = Method::from_node(child, content) {
                        t.methods.insert(method.name.clone(), method);
                    }
                } else if child.kind() == "use_declaration" {
                    let trait_names = clause_fqn_names(child, content, scope, ns_store);
                    t.traits_used.extend(trait_names.clone());
                    dependencies.extend(trait_names);
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.kind().ends_with("_clause") {
            continue;
        }

        let names = clause_fqn_names(child, content, scope, ns_store);
        if child.kind() == "base_clause" {
            t.parent_classes.extend(names.clone());
        } else if child.kind() == "class_interface_clause" {
            t.implemented_interfaces.extend(names.clone());
        } else {
            panic!("unsupported `_clause` = `{}`", child.kind());
        }

        dependencies.extend(names);
    }

    if t.name != "" {
        let ns = if let Some(ns) = &scope.ns {
            let mut ns = ns.clone();
            ns.push(Arc::from(t.name.as_str()));
            ns
        } else {
            PhpNamespace::empty()
        };
        types.0.insert(
            ns,
            CustomTypeMeta {
                t: CustomType::Class(t),
                markup,
                src_range: node.range(),
            },
        );
    }
}

#[cfg(test)]
mod test {
    use tree_sitter::Parser;
    use tree_sitter_php::language_php;

    use crate::php_namespace::SegmentPool;
    use crate::scope::Scope;
    use crate::types::{
        Array, CustomType, CustomTypesDatabase, Nullable, Scalar, Type, Visibility,
    };

    fn parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&language_php())
            .expect("error loading PHP grammar");

        parser
    }

    #[test]
    fn ns_usage() {
        let src = "<?php
        namespace Foo;

        use Foo\\Bar, Foo\\Bar\\Blah;
        use Foo\\Bah as Cat;";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let mut pool = SegmentPool::new();
        let diags = super::walk(root_node, src, &mut pool);
        assert!(diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
        assert_eq!(pool.0.len(), 4, "pool = {:?}", pool.0);
    }

    #[test]
    fn same_ns() {
        let src = "<?php
        namespace Foo;

        use Foo\\Bar, Foo\\Bar\\Blah;
        use Foo\\Bah as Bar;";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let mut pool = SegmentPool::new();
        let diags = super::walk(root_node, src, &mut pool);
        assert_eq!(diags.len(), 1, "src = {}\ndiags = {:?}", src, diags);
        assert_eq!(pool.0.len(), 4, "pool = {:?}", pool.0);
    }

    #[test]
    fn param_is_superglobal() {
        let src = "<?php
        function foo(int $_GET) {}";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let diags = super::walk(root_node, src, &mut SegmentPool::new());
        assert!(diags.len() == 1, "src = {}\ndiags = {:?}", src, diags);
    }

    #[test]
    fn defined_superglobals() {
        let src = "<?php var_dump($_GET);";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let diags = super::walk(root_node, src, &mut SegmentPool::new());
        assert!(diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
    }

    #[test]
    fn class_decl_in_types_db() {
        let src = "<?php
        namespace Foo\\Bar;

        /**
         * hello world
         */
        class Baz {
            protected static ?array $someArray = null;
            public static function bar(): string {}
        }
        ";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let mut types = CustomTypesDatabase::new();
        let mut pool = SegmentPool::new();
        let deps = super::injest_types(root_node, src, &mut pool, &mut types);
        assert!(deps.is_empty(), "src = {}\ndeps = {:?}", src, deps);
        assert_eq!(types.0.len(), 1);

        let query = pool.intern_str("Foo\\Bar\\Baz");
        let meta = types.0.get(&query).unwrap();
        let c = match &meta.t {
            CustomType::Class(c) => c,
            _ => unreachable!("type should only be a class"),
        };
        assert_eq!(&c.name, "Baz");
        assert!(meta.markup.as_ref().unwrap().contains("hello world"));
        let m = c.methods.get("bar").unwrap();
        assert_eq!(&m.name, "bar");
        assert_eq!(m.return_type, Type::Scalar(Scalar::String));
        assert_eq!(m.r#abstract, false);
        assert_eq!(m.r#static, true);
        assert_eq!(m.visibility, Visibility::Public);
        let p = c.properties.get("$someArray").unwrap();
        assert_eq!(p.t, Type::Nullable(Nullable(Box::new(Type::Array))));
    }

    #[test]
    fn class_decl_extends_with_ns() {
        let src = "<?php
        namespace Foo\\Bar;

        use Foo\\Pa;
        use Foo\\Sa\\Trait1;
        use Foo\\Sa\\Trait2;

        class Baz extends Ta, \\Foo\\Da {
            use Trait1, Pa\\Trait2;
        }
        ";
        let tree = parser().parse(src, None).unwrap();
        let root_node = tree.root_node();
        let mut types = CustomTypesDatabase::new();
        let mut pool = SegmentPool::new();
        let deps = super::injest_types(root_node, src, &mut pool, &mut types);

        let baz = types.0.get(&pool.intern_str("Foo\\Bar\\Baz")).unwrap();
        let baz_t = match &baz.t {
            CustomType::Class(c) => c,
            _ => unreachable!(),
        };

        assert!(baz_t
            .parent_classes
            .contains(&pool.intern_str("Foo\\Bar\\Ta")));
        assert!(baz_t.parent_classes.contains(&pool.intern_str("Foo\\Da")));
        assert!(baz_t
            .traits_used
            .contains(&pool.intern_str("Foo\\Sa\\Trait1")));
        assert!(baz_t
            .traits_used
            .contains(&pool.intern_str("Foo\\Pa\\Trait2")));

        assert_eq!(deps.len(), 4);
        assert!(deps.contains(&pool.intern_str("Foo\\Bar\\Ta")));
        assert!(deps.contains(&pool.intern_str("Foo\\Da")));
        assert!(deps.contains(&pool.intern_str("Foo\\Sa\\Trait1")));
        assert!(deps.contains(&pool.intern_str("Foo\\Pa\\Trait2")));
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
        let mut diags = vec![];
        super::walk_statement(stmt1, src, &mut SegmentPool::new(), &mut scope, &mut diags);
        assert!(diags.is_empty());
        assert_eq!(10, scope.symbols.len());

        let stmt2 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt2.kind());
        diags = vec![];
        super::walk_statement(stmt2, src, &mut SegmentPool::new(), &mut scope, &mut diags);
        assert_eq!(1, diags.len());
        let diag = &diags[0];
        assert_eq!("undefined variable $var2", &diag.message);
        assert_eq!(11, scope.symbols.len());

        assert!(scope.symbols.contains("$var1"));
        assert!(scope.symbols.contains("$var2"));

        let stmt3 = iter.next().unwrap();
        assert_eq!("expression_statement", stmt3.kind());
        diags = vec![];
        super::walk_statement(stmt3, src, &mut SegmentPool::new(), &mut scope, &mut diags);
        assert_eq!(1, diags.len());
        let diag = &diags[0];
        assert_eq!("undefined variable $var4", &diag.message);
        assert_eq!(13, scope.symbols.len());

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
            "<?php
            array_map(function () {
                $a = 3;
                return $a;
            }, []);",
            "<?php
            $x = 1;
            array_map(function() use ($x) {
                $a = $x;
                return $a;
            }, []);",
            "<?php
            $x = $_GET['x'];
            switch ($x) {
            case 3:
            case 4:
                $y = 300;
                break;
            case 6:
            default:
                $y = 400;
                break;
            }

            $z = $y;",
            "<?php
            $l = [1, 2, 3];
            $sum = 0;
            foreach ($l as &$item) {
                $sum += $item;
                $item = 0;
            }",
            "<?php
            $a = 3;
            $b = &$a;",
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::walk(root_node, src, &mut SegmentPool::new());
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
            "<?php
            echo $x;",
        ];

        for src in srcs {
            let tree = parser().parse(src, None).unwrap();
            let root_node = tree.root_node();
            let diags = super::walk(root_node, src, &mut SegmentPool::new());
            assert!(!diags.is_empty(), "src = {}\ndiags = {:?}", src, diags);
        }
    }
}
