use lsp_types::{WorkspaceFolder, Uri};

use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    workspace_folders: Vec<WorkspaceFolder>
}

impl Config {
    pub fn new(mut workspace_folders: Vec<WorkspaceFolder>, root_uri: Option<Uri>) -> Self {
        if workspace_folders.is_empty() {
            if let Some(root_uri) = root_uri {
                workspace_folders.push(WorkspaceFolder {
                    uri: root_uri.clone(),
                    name: root_uri.to_string(),
                });
            }
        }

        for folder in workspace_folders {
            folder.uri.path().is_absolute()
        }

        Config {
            workspace_folders,
        }
    }
}
