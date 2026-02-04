use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use cf_speedtest::{CTRL_C_PRESSED, TestResults};

use cf_speedtest::UserArgs;

use cf_speedtest::{print_results_table, print_test_preamble};
use cf_speedtest::{run_download_test, run_upload_test};


fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let config: UserArgs = argh::from_env();
    config.validate().expect("Invalid arguments");

    let results = Arc::new(Mutex::new(TestResults::default()));
    let results_clone = Arc::clone(&results);

    // Set up CTRL-C handler
    ctrlc::set_handler(move || {
        CTRL_C_PRESSED.store(true, Ordering::Relaxed);
        println!("\n\nReceived CTRL-C, printing current results...");
        if let Ok(current_results) = results_clone.lock() {
            print_results_table(&current_results);
        }
        std::process::exit(0);
    })
    .expect("Error setting CTRL-C handler");

    print_test_preamble();

    if !config.upload_only {
        run_download_test(&config, Arc::clone(&results), Arc::new(AtomicBool::new(false)));
    }

    if !config.download_only {
        println!("Starting upload tests...");
        run_upload_test(&config, Arc::clone(&results), Arc::new(AtomicBool::new(false)));
    }

    // Print final results
    if let Ok(final_results) = results.lock() {
        print_results_table(&final_results);
    };
}
