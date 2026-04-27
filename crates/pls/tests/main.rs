//! Integration tests go here
use lsp_server::Connection;

use pls::global_state::GlobalState;
use std::thread;
use std::time::Duration;

mod support;

const STUBS_FILENAME: &'static str = "./phpstorm-stubs/PhpStormStubsMap.php";

#[derive(Debug)]
enum QuittingState {
    GracefulShutdown,
    ThreadTimeout(Duration),
}

#[test]
fn minimal_config_that_quits() {
    let (connection, client) = Connection::memory();
    let mut client = support::FakeClient::new(client);
    let (tx, rx) = crossbeam_channel::bounded(2);
    let tx2 = tx.clone();
    thread::spawn(move || {
        let mut state = GlobalState::new(STUBS_FILENAME, connection).expect("global state initialization");
        state.main_loop();

        let _ = tx2.send(QuittingState::GracefulShutdown);
    });

    client.initialize();
    client.shutdown();

    let wait_time = Duration::from_secs(2);
    thread::spawn(move || {
        thread::sleep(wait_time);

        let _ = tx.send(QuittingState::ThreadTimeout(wait_time));
    });

    match rx.recv() {
        Ok(QuittingState::GracefulShutdown) => {}
        Ok(QuittingState::ThreadTimeout(t)) => {
            panic!("Timeout {t:?} passed and main loop still hasn't stopped!");
        }
        Err(e) => {
            panic!("Error occurred trying to receive from shutdown channel: {:?}", e);
        }
    }
}
