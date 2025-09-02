use std::env;

use tower_lsp_server::{LspService, Server};

mod analyze;
mod backend;
mod code_action;
mod compat;
mod composer;
mod diagnostics;
mod file;
mod messages;
mod php_namespace;
mod scope;
mod stubs;
mod types;

const VERSION_ARG: &'static str = "--version";

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    // no need to include `clap` when this suffices for the moment
    let mut stubs_filename = None;
    for (i, arg) in env::args().enumerate() {
        if i == 0 {
            continue;
        }

        if &arg == VERSION_ARG {
            println!(
                "{} version {}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            );
            return;
        } else {
            stubs_filename = Some(arg);
            break;
        }
    }

    match stubs_filename {
        None => {
            println!("error: missing argument: location of stubs file; e.g.: `phplsp phpstorm-stubs/PhpStormStubsMap.php`");
            return;
        }
        Some(stubs_filename) => {
            let (service, socket) =
                LspService::new(|client| backend::Backend::new(stubs_filename, client).unwrap());
            Server::new(stdin, stdout, socket).serve(service).await;
        }
    }
}
