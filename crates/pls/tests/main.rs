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
