use lsp_types::*;

use std::time::Duration;

mod support;

const STUBS_FILENAME: &'static str = "./phpstorm-stubs/PhpStormStubsMap.php";

#[test]
fn minimal_config_that_quits() {
    support::run_with(
        support::TestConfig {
            stubs_filename: STUBS_FILENAME,
            max_test_duration: Duration::from_secs(2),
        },
        |_client| {},
    );
}

#[test]
fn code_actions_on_opened_file() {
    support::run_with(
        support::TestConfig {
            stubs_filename: STUBS_FILENAME,
            max_test_duration: Duration::from_secs(2),
        },
        |client| {
            use std::str::FromStr as _;

            let uri = Uri::from_str("file:///tmp/file.php").unwrap();

            client.notify::<notification::DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "php".to_string(),
                    version: 1,
                    text: "<?php echo 'hello world'; ?>

                        <?php echo 1 + 2 + 3; ?>"
                        .to_string(),
                },
            });
            let id = client.request::<request::CodeActionRequest>(CodeActionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                range: Range {
                    start: Position {
                        line: 1,
                        character: 1,
                    },
                    end: Position {
                        line: 1,
                        character: 1,
                    },
                },
                context: CodeActionContext {
                    diagnostics: Vec::new(),
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: WorkDoneProgressParams {
                    work_done_token: None,
                },
                partial_result_params: PartialResultParams {
                    partial_result_token: None,
                },
            });

            let code_action_response = client
                .next_response(id, 100)
                .expect("code action response from server");
            let code_action_result: Vec<CodeAction> = serde_json::from_value(
                code_action_response
                    .result
                    .expect("result field from response"),
            )
            .expect("code action from server");

            assert!(!code_action_result.is_empty());

            let phpecho_code_action = code_action_result
                .iter()
                .find(|action| action.title.contains("`<?php echo`"))
                .expect("code action converting <?php echo");
            let id =
                client.request::<request::CodeActionResolveRequest>(phpecho_code_action.to_owned());

            let resolution_response = client
                .next_response(id, 100)
                .expect("code action resolution response from server");
            let resolution_result: CodeAction = serde_json::from_value(
                resolution_response
                    .result
                    .expect("result field from response"),
            )
            .expect("code action from server");

            let DocumentChanges::Edits(edits) =
                &resolution_result.edit.unwrap().document_changes.unwrap()
            else {
                panic!("no document changes edits found");
            };

            assert_eq!(1, edits.len());

            let edit = &edits[0];

            assert_eq!(&uri, &edit.text_document.uri);
            assert_eq!(
                2,
                edit.edits.len(),
                "expected 2 edits for the 2 php echoes in the document; found: {:?}",
                edit.edits
            );
        },
    );
}
