use lsp_server::Connection;

use std::env;

mod compat;
mod config;
mod diagnostics;
mod file;
mod global_state;
mod handlers;
mod messages;
mod scope;
mod stubs;
mod registry;

use global_state::GlobalState;

const VERSION_ARG: &'static str = "--version";

fn main() -> anyhow::Result<()> {
    colog::init();

    // no need to include `clap` when this will suffice
    let mut stubs_filename = None;
    for (i, arg) in env::args().enumerate() {
        if i == 0 {
            continue;
        }

        if &arg == VERSION_ARG {
            log::info!(
                "{} version {}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        } else {
            stubs_filename = Some(arg);
            break;
        }
    }

    match stubs_filename {
        None => {
            log::error!(
                "missing argument: location of stubs file; e.g.: `{} phpstorm-stubs/PhpStormStubsMap.php`",
                env!("CARGO_PKG_NAME")
            );
            return Ok(());
        }
        Some(stubs_filename) => {
            log::debug!("starting server version {}", env!("CARGO_PKG_VERSION"));

            let (connection, io_threads) = Connection::stdio();
            let mut state = GlobalState::new(&stubs_filename, connection).expect("global state initialization");

            state.main_loop();
            io_threads.join()?;
        }
    }

    Ok(())
}
