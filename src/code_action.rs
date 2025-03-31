use tower_lsp::lsp_types::*;

use regex::Regex;

use std::sync::OnceLock;

use crate::file::offset_to_position;

fn phpecho_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<\?php\s+echo\s+([^;]+);\s*\?>").unwrap())
}

pub fn changes_phpecho(uri: &Url, contents: &str, version: i32) -> Option<DocumentChanges> {
    let mut edits = vec![];
    let text_document = OptionalVersionedTextDocumentIdentifier {
        uri: uri.clone(),
        version: Some(version),
    };

    let re = phpecho_re();
    for captures in re.captures_iter(contents) {
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
    use tower_lsp::lsp_types::*;

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
    fn will_change_phpechos() {
        let contents = "<?php   echo   addslashes('evil evil')  ;    ?>


            <?php echo 34; ?>";
        let uri = Url::parse("https://google.ca").unwrap();
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
