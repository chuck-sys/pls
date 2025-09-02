use tower_lsp_server::lsp_types::*;

use tree_sitter::{InputEdit, Node, Parser, Query, QueryCursor, StreamingIterator, Tree};
use tree_sitter_php::LANGUAGE_PHP;
use tree_sitter_phpdoc::language as language_phpdoc;

use std::error::Error;
use std::fmt::Display;
use std::sync::{Arc, OnceLock};

use crate::compat::to_point;
use crate::php_namespace::PhpNamespace;
use crate::types::Type;

pub struct FileData {
    pub contents: String,
    pub php_tree: Tree,
    pub comments_tree: Tree,
    pub version: i32,
}

#[derive(Debug)]
pub enum FileError {
    InvalidFileRange(Range),
}

impl Error for FileError {}

impl Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::InvalidFileRange(range) => {
                write!(f, "file range {:?} isn't a valid byte offset", range)
            }
        }
    }
}

impl FileData {
    pub fn change(&mut self, event: TextDocumentContentChangeEvent) -> Result<(), FileError> {
        if let Some(r) = event.range {
            if let (Some(start_byte), Some(end_byte)) = (
                byte_offset(&self.contents, &r.start),
                byte_offset(&self.contents, &r.end),
            ) {
                let input_edit = InputEdit {
                    start_byte,
                    old_end_byte: end_byte,
                    new_end_byte: start_byte + event.text.len(),
                    start_position: to_point(&r.start),
                    old_end_position: to_point(&r.end),
                    new_end_position: {
                        let mut row = r.start.line as usize;
                        let mut column = r.start.character as usize;

                        for c in event.text.chars() {
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
                self.php_tree.edit(&input_edit);
                self.comments_tree.edit(&input_edit);
                self.contents
                    .replace_range(start_byte..end_byte, &event.text);
            } else {
                return Err(FileError::InvalidFileRange(r));
            }
        } else {
            self.contents = event.text.clone();
        }

        Ok(())
    }

    /// Type definitions.
    ///
    /// Includes the following:
    /// - Classes and their methods
    /// - Trait definitions
    /// - Interface definitions
    /// - Method signatures
    /// - Function signatures
    pub fn types(&self) -> Vec<Type> {
        let mut ts = Vec::new();
        let root_node = self.php_tree.root_node();
        let mut cursor = root_node.walk();
        if !cursor.goto_first_child() {
            return Vec::new();
        }

        let mut stack = Vec::with_capacity(50);
        stack.push(cursor.node());
        while cursor.goto_next_sibling() {
            stack.push(cursor.node());
        }

        while let Some(node) = stack.pop() {
            let kind = node.kind();
            let prev_node = node.prev_sibling();

            if kind == "class_declaration" {
            } else if kind == "function_declaration" {
            } else if kind == "trait_declaration" {
            } else if kind == "interface_declaration" {
            }

            for c in node.children(&mut cursor) {
                stack.push(c);
            }
        }

        ts
    }
}

fn comment_query() -> &'static Query {
    static Q: OnceLock<Query> = OnceLock::new();
    Q.get_or_init(|| Query::new(&LANGUAGE_PHP.into(), "(comment)").unwrap())
}

fn get_comment_ranges(node: Node<'_>, contents: &str) -> Vec<tree_sitter::Range> {
    let mut ranges = Vec::new();
    let query = comment_query();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, node, contents.as_bytes());
    while let Some(m) = captures.next() {
        for c in m.0.captures.iter() {
            ranges.push(c.node.range());
        }
    }

    ranges
}

pub fn parse(
    contents: &str,
    (php_tree, doc_tree): (Option<&Tree>, Option<&Tree>),
) -> (Tree, Tree) {
    let mut php_parser = Parser::new();
    php_parser.set_language(&LANGUAGE_PHP.into()).unwrap();
    let mut phpdoc_parser = Parser::new();
    phpdoc_parser.set_language(&language_phpdoc()).unwrap();
    let php_tree = php_parser.parse(contents, php_tree).unwrap();

    let comment_ranges = get_comment_ranges(php_tree.root_node(), contents);
    phpdoc_parser.set_included_ranges(&comment_ranges).unwrap();

    let doc_tree = phpdoc_parser.parse(contents, doc_tree).unwrap();

    (php_tree, doc_tree)
}

/// Convert character offset into a position.
///
/// If the offset is outside the contents given, return the last position of the file.
pub fn offset_to_position(contents: &str, mut offset: usize) -> Position {
    let mut line = 0;
    let mut character = 0;
    for c in contents.chars() {
        if offset == 0 {
            return Position { line, character };
        }

        if c == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }

        offset -= 1;
    }

    Position { line, character }
}

/// Get byte offset given some row and column position in a file.
///
/// For example, line 0 character 0 should have offset of 0 (0-indexing). We don't check that the
/// column is within the current line (e.g. line 0 character 2000 gives offset of 2000 even if the
/// line isn't that long).
///
/// Return None if the position is invalid (i.e. not in the file, out of range of current line,
/// etc.)
pub fn byte_offset(text: &str, r: &Position) -> Option<usize> {
    let mut current_line = 0;
    let mut current_offset = 0usize;

    for c in text.chars() {
        if current_line == r.line {
            return Some(current_offset + r.character as usize);
        }

        if c == '\n' {
            current_line += 1;
        }

        current_offset += 1;
    }

    None
}

#[cfg(test)]
mod test {
    use tower_lsp_server::lsp_types::*;

    use super::byte_offset;

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
    fn valid_byte_offsets() {
        let valids = [
            (
                Position {
                    line: 0,
                    character: 0,
                },
                0usize,
            ),
            (
                Position {
                    line: 1,
                    character: 0,
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
    fn invalid_byte_offsets() {
        let invalids = [Position {
            line: 200,
            character: 10,
        }];

        let s = SOURCE.to_string();
        for invalid_position in invalids {
            assert_eq!(None, byte_offset(&s, &invalid_position));
        }
    }
}
