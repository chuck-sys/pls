use lsp_types::*;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};
use tree_sitter_php::LANGUAGE_PHP;
use regex::Regex;

use std::sync::LazyLock;

use crate::compat::to_point;
use crate::file::offset_to_position;
use crate::global_state::FileInfo;

pub const PHPECHO_TITLE: &'static str = "Convert `<?php echo` into `<?=`";
pub const TMPLSTR_TITLE: &'static str = "Use template string";

#[derive(Serialize, Deserialize)]
pub struct PhpEchoParams {
    pub uri: Uri,
}

static PHPECHO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<\?php\s+echo\s+([^;]+);\s*\?>").unwrap());
static CONCAT_STR_QUERY: LazyLock<Query> = 
    LazyLock::new(|| Query::new(&LANGUAGE_PHP.into(), r#"(binary_expression operator: ".") @concat_expr"#).unwrap());

fn is_concat_expr(content: &str, node: &Node<'_>) -> bool {
    if node.kind() != "binary_expression" {
        return false;
    }

    let Some(operator) = node.child_by_field_name("operator") else {
        return false;
    };

    content[operator.byte_range()] == *"."
}

fn closest_concat_expr<'a>(file_info: &'a FileInfo, range: &Range) -> Option<Node<'a>> {
    let root_node = file_info.php_ast.root_node();
    let Some(node) = root_node.descendant_for_point_range(to_point(&range.start), to_point(&range.end)) else {
        return None
    };

    match node.kind() {
        "binary_expression" => is_concat_expr(&file_info.content, &node).then_some(node),
        "string" | "encapsed_string" => {
            let Some(parent) = node.parent() else {
                return None;
            };

            is_concat_expr(&file_info.content, &parent).then_some(parent)
        }
        "string_content" => {
            let Some(parent) = node.parent().and_then(|n| n.parent()) else {
                return None;
            };

            is_concat_expr(&file_info.content, &parent).then_some(parent)
        }
        _ => None,
    }
}

fn outermost_concat_expr(node: Node<'_>) -> Node<'_> {
    node
}

pub fn can_change_to_tmplstr(file_info: &FileInfo, range: &Range) -> bool {
    closest_concat_expr(file_info, range).is_some()
}

pub fn changes_tmplstr(uri: &Uri, file_info: &FileInfo, range: &Range) -> Option<DocumentChanges> {
    let Some(starting_node) = closest_concat_expr(file_info, range).map(outermost_concat_expr) else {
        return None;
    };

    let mut edits = Vec::new();
    let text_document = OptionalVersionedTextDocumentIdentifier {
        uri: uri.clone(),
        version: Some(file_info.version),
    };
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&CONCAT_STR_QUERY, starting_node, file_info.content.as_bytes());

    while let Some((m, _)) = captures.next() {
        for c in m.captures.iter() {
        }
    }

    Some(DocumentChanges::Edits(vec![TextDocumentEdit {
        text_document,
        edits,
    }]))
}

pub fn changes_phpecho(uri: &Uri, contents: &str, version: i32) -> Option<DocumentChanges> {
    let mut edits = vec![];
    let text_document = OptionalVersionedTextDocumentIdentifier {
        uri: uri.clone(),
        version: Some(version),
    };

    for captures in PHPECHO_RE.captures_iter(contents) {
        let m = captures.get(0).unwrap();
        let range = Range {
            start: offset_to_position(contents, m.start()),
            end: offset_to_position(contents, m.end()),
        };

        let trimmed = captures.get(1).unwrap().as_str().trim_end();
        let new_text = format!("<?= {} ?>", trimmed);
        edits.push(OneOf::Left(TextEdit { range, new_text }));
    }

    Some(DocumentChanges::Edits(vec![TextDocumentEdit {
        text_document,
        edits,
    }]))
}

#[cfg(test)]
mod test {
    use lsp_types::*;
    use std::str::FromStr;

    use super::changes_phpecho;

    macro_rules! unwrap_enum {
        ($value:expr, $variant:path) => {
            match $value {
                $variant(inner) => inner,
                _ => unreachable!(),
            }
        };
    }

    #[test]
    fn will_change_tmplstr() {
        let contents = "<?php 'abc' . $i . 'def'; ?>";
        let uri = Uri::from_str("file:///tmp/file.php").unwrap();
    }

    #[test]
    fn will_change_phpechos() {
        let contents = "<?php   echo   addslashes('evil evil')  ;    ?>


            <?php echo 34; ?>";
        let uri = Uri::from_str("https://google.ca").unwrap();
        let edits = unwrap_enum!(
            changes_phpecho(&uri, &contents, 1).unwrap(),
            DocumentChanges::Edits
        )[0]
        .edits
        .clone();
        let edit1 = unwrap_enum!(&edits[0], OneOf::Left);
        let edit2 = unwrap_enum!(&edits[1], OneOf::Left);

        assert_eq!(&edit1.new_text, "<?= addslashes('evil evil') ?>");
        assert_eq!(
            &edit1.range.start,
            &Position {
                line: 0,
                character: 0,
            }
        );
        assert_eq!(
            &edit1.range.end,
            &Position {
                line: 0,
                character: 47,
            }
        );
        assert_eq!(&edit2.new_text, "<?= 34 ?>");
        assert_eq!(
            &edit2.range.start,
            &Position {
                line: 3,
                character: 12,
            }
        );
        assert_eq!(
            &edit2.range.end,
            &Position {
                line: 3,
                character: 29,
            }
        );
    }
}
