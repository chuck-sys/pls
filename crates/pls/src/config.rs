use lsp_types::{WorkspaceFolder, Uri};

use std::str::FromStr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    stubs_filename: PathBuf,
    workspace_folders: Vec<PathBuf>,
}

impl Config {
    pub fn new(mut workspace_folders: Vec<WorkspaceFolder>, root_uri: Option<Uri>, stubs_filename: PathBuf) -> Self {
        if workspace_folders.is_empty() {
            if let Some(root_uri) = root_uri {
                workspace_folders.push(WorkspaceFolder {
                    uri: root_uri.clone(),
                    name: root_uri.to_string(),
                });
            }
        }

        Config {
            stubs_filename,
            workspace_folders: workspace_folders
                .into_iter()
                .filter_map(|f| PathBuf::from_str(&f.uri.to_string()).ok())
                .collect(),
        }
    }
}
