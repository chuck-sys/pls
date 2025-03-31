use tower_lsp::lsp_types::*;

use tree_sitter::Tree;

pub struct FileData {
    pub contents: String,
    pub php_tree: Tree,
    pub comments_tree: Tree,
    pub version: i32,
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
    use tower_lsp::lsp_types::*;

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
