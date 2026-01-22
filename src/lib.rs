use std::sync::{Arc, Mutex, atomic::AtomicBool};

pub use speed_test::{run_download_test, run_upload_test};
pub use print::{print_results_table, print_test_preamble};
pub use args::UserArgs;

use crate::speed_test::compute_statistics;


mod args;
mod agent;
mod speed_test;
mod raw_socket;
mod table;
mod print;
mod locations;
#[cfg(test)]
mod tests;


pub static CTRL_C_PRESSED: AtomicBool = AtomicBool::new(false);

static CLOUDFLARE_SPEEDTEST_DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down?measId=0";
static CLOUDFLARE_SPEEDTEST_UPLOAD_URL: &str = "https://speed.cloudflare.com/__up?measId=0";
static CLOUDFLARE_SPEEDTEST_SERVER_URL: &str =
    "https://speed.cloudflare.com/__down?measId=0&bytes=0";
static CLOUDFLARE_SPEEDTEST_CGI_URL: &str = "https://speed.cloudflare.com/cdn-cgi/trace";
static OUR_USER_AGENT: &str = concat!(
    "cf_speedtest (",
    env!("CARGO_PKG_VERSION"),
    ") https://github.com/12932/cf_speedtest"
);

static CONNECT_TIMEOUT_MILLIS: u64 = 9600;
static LATENCY_TEST_COUNT: u8 = 8;
static NEW_METAL_SLEEP_MILLIS: u32 = 250;



#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SpeedTestResult {
    pub download_mbps: f64,
    pub upload_mbps: f64
}

#[derive(Clone, Default)]
pub struct TestResults {
    pub down_measurements: Vec<usize>,
    pub up_measurements: Vec<usize>,
    pub download_completed: bool,
    pub upload_completed: bool,
}

pub fn run_speed_test(expedite: bool) -> anyhow::Result<SpeedTestResult> {
    let mut config = UserArgs::default();
    if expedite {
        config.test_duration_seconds = 3;
    }

    let results = Arc::new(Mutex::new(TestResults::default()));

    if !config.upload_only {
        run_download_test(&config, Arc::clone(&results));
    }

    if !config.download_only {
        run_upload_test(&config, Arc::clone(&results));
    }


    let results = results.lock().map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let mut down_measurements = results.down_measurements.clone();
    let mut up_measurements = results.up_measurements.clone();

    let (download_median, _, _, _, _, _) = compute_statistics(&mut down_measurements);
    let (upload_median, _, _, _, _, _) = compute_statistics(&mut up_measurements);

    Ok(SpeedTestResult {
        download_mbps: download_median / 1_000_000.0 * 8.0,
        upload_mbps: upload_median / 1_000_000.0 * 8.0
    })
}
