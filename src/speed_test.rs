use std::{sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}}, thread::JoinHandle, time::{Instant, SystemTime, UNIX_EPOCH}};

use ureq::Agent;

use crate::{CLOUDFLARE_SPEEDTEST_CGI_URL, CLOUDFLARE_SPEEDTEST_DOWNLOAD_URL, CLOUDFLARE_SPEEDTEST_SERVER_URL, CLOUDFLARE_SPEEDTEST_UPLOAD_URL, CTRL_C_PRESSED, LATENCY_TEST_COUNT, NEW_METAL_SLEEP_MILLIS, TestResults, agent::create_configured_agent, args::UserArgs, raw_socket::RawDownloadConnection};


type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;


impl std::io::Read for UploadHelper {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // upload is finished, or we are exiting
        if self.byte_ctr.load(Ordering::SeqCst) >= self.bytes_to_send
            || self.exit_signal.load(Ordering::SeqCst)
        {
            return Ok(0);
        }

        buf.fill(1);

        self.byte_ctr.fetch_add(buf.len(), Ordering::SeqCst);
        self.total_uploaded_counter
            .fetch_add(buf.len(), Ordering::SeqCst);
        Ok(buf.len())
    }
}

struct UploadHelper {
    bytes_to_send: usize,
    byte_ctr: Arc<AtomicUsize>,
    total_uploaded_counter: Arc<AtomicUsize>,
    exit_signal: Arc<AtomicBool>,
}

fn get_secs_since_unix_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// Default test duration + a little bit more if we have extra threads
fn get_test_time(test_duration_seconds: u64, thread_count: u32) -> u64 {
    if thread_count > 4 {
        return test_duration_seconds + (thread_count as u64 - 4) / 4;
    }

    test_duration_seconds
}

/* Given n bytes, return
     a: unit of measurement in sensible form of bytes
     b: unit of measurement in sensible form of bits
 i.e 12939428 	-> (12.34 MB, 98.76 Mb)
     814811 	-> (795.8 KB, 6.36 Mb)
*/
pub fn get_appropriate_byte_unit(bytes: u64) -> (String, String) {
    const UNITS: [&str; 5] = [" ", "K", "M", "G", "T"];
    const KILOBYTE: f64 = 1024.0;

    let mut bytes = bytes as f64;
    let mut level = 0;

    while bytes >= KILOBYTE && level < UNITS.len() - 1 {
        bytes /= KILOBYTE;
        level += 1;
    }

    let byte_unit = UNITS[level];
    let mut bits = bytes * 8.0;
    let mut bit_unit = byte_unit.to_ascii_lowercase();

    if bits >= 1000.0 {
        bits /= 1000.0;
        bit_unit = match byte_unit {
            " " => "k",
            "K" => "m",
            "M" => "g",
            "G" => "t",
            "T" => "p",
            _ => "?",
        }
        .to_string();
    }

    (
        format!("{bytes:.2} {byte_unit}B"),
        format!("{bits:.2} {bit_unit}b"),
    )
}

pub fn get_appropriate_byte_unit_rate(bytes: u64) -> (String, String) {
    let (a, b) = get_appropriate_byte_unit(bytes);
    (format!("{a}/s"), format!("{b}it/s"))
}

fn get_appropriate_buff_size(speed: usize) -> u64 {
    match speed {
        0..=1000 => 4,
        1001..=10000 => 32,
        10001..=100000 => 512,
        100001..=1000000 => 4096,
        _ => 16384,
    }
}

// Use cloudflare's cdn-cgi endpoint to get our ip address country
pub fn get_our_ip_address_country() -> Result<String> {
    let mut resp = ureq::get(CLOUDFLARE_SPEEDTEST_CGI_URL).call()?;
    let body: String = resp.body_mut().read_to_string()?;

    for line in body.lines() {
        if let Some(loc) = line.strip_prefix("loc=") {
            return Ok(loc.to_string());
        }
    }

    panic!(
        "Could not find loc= in cdn-cgi response\n
			Please update to the latest version and make a Github issue if the issue persists"
    );
}

// Get http latency by requesting the cgi endpoint 8 times
// and taking the fastest
pub fn get_download_server_http_latency() -> Result<std::time::Duration> {
    let start = Instant::now();

    let my_agent = create_configured_agent();
    let mut latency_vec = Vec::new();

    for _ in 0..LATENCY_TEST_COUNT {
        // if vec length 2 or greater and we've spent a lot of time
        // 	calculating latency, exit early (we could be on satellite or sumthin)
        if latency_vec.len() >= 2 && start.elapsed() > std::time::Duration::from_secs(1) {
            break;
        }

        let now = Instant::now();

        let _response = my_agent
            .get(CLOUDFLARE_SPEEDTEST_CGI_URL)
            .call()?
            .body_mut()
            .read_to_string();

        let total_time = now.elapsed();
        latency_vec.push(total_time);
    }

    let best_time = latency_vec.iter().min().unwrap().to_owned();
    Ok(best_time)
}

// return all cloufdlare headers from a request
pub fn get_download_server_info() -> Result<std::collections::HashMap<String, String>> {
    let mut server_headers = std::collections::HashMap::new();
    let resp = ureq::get(CLOUDFLARE_SPEEDTEST_SERVER_URL)
        .call()
        .expect("Failed to get server info");

    // Using headers() instead of headers_names()
    for header in resp.headers() {
        let key_str = header.0.as_str();
        if key_str.starts_with("cf-") {
            server_headers.insert(
                key_str.to_string(),
                header.1.to_str().unwrap_or_default().to_string(),
            );
        }
    }

    Ok(server_headers)
}

pub fn get_current_timestamp() -> String {
    let now = chrono::Local::now();

    format!("{} {}", now.format("%Y-%m-%d %H:%M:%S"), now.format("%Z"))
}

pub fn upload_test(
    bytes: usize,
    total_up_bytes_counter: &Arc<AtomicUsize>,
    _current_speed: &Arc<AtomicUsize>,
    exit_signal: &Arc<AtomicBool>,
) -> Result<()> {
    let agent: Agent = create_configured_agent();

    loop {
        let upload_helper = UploadHelper {
            bytes_to_send: bytes,
            byte_ctr: Arc::new(AtomicUsize::new(0)),
            total_uploaded_counter: total_up_bytes_counter.clone(),
            exit_signal: exit_signal.clone(),
        };

        let body = ureq::SendBody::from_owned_reader(upload_helper);

        let resp = match agent
            .post(CLOUDFLARE_SPEEDTEST_UPLOAD_URL)
            .header("Content-Type", "text/plain;charset=UTF-8")
            .send(body)
        {
            Ok(resp) => resp,
            Err(err) => {
                if !CTRL_C_PRESSED.load(Ordering::Relaxed) {
                    log::error!("Error in upload thread: {err}");
                }
                return Ok(());
            }
        };

        // Process the response
        let _ = std::io::copy(&mut resp.into_body().into_reader(), &mut std::io::sink());

        if exit_signal.load(Ordering::Relaxed) {
            return Ok(());
        }
    }
}

// download some bytes from cloudflare using raw encrypted byte reading
pub fn download_test(
    bytes_to_request: usize,
    total_bytes_counter: &Arc<AtomicUsize>,
    current_down_speed: &Arc<AtomicUsize>,
    exit_signal: &Arc<AtomicBool>,
) -> Result<()> {
    // Keep making new requests until exit_signal is set
    loop {
        // exit if we have passed deadline
        if exit_signal.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Establish connection, perform TLS handshake, send HTTP request
        let mut conn = match RawDownloadConnection::connect(
            CLOUDFLARE_SPEEDTEST_DOWNLOAD_URL,
            bytes_to_request,
        ) {
            Ok(conn) => conn,
            Err(err) => {
                if !CTRL_C_PRESSED.load(Ordering::Relaxed) {
                    log::error!("Error in download thread: {err}");
                }
                return Ok(());
            }
        };

        let mut total_bytes_sank: usize = 0;

        // Read from this connection until it's exhausted
        loop {
            // exit if we have passed deadline
            if exit_signal.load(Ordering::Relaxed) {
                return Ok(());
            }

            // if we are fast, take big chunks
            // if we are slow, take small chunks
            let current_recv_buff =
                get_appropriate_buff_size(current_down_speed.load(Ordering::Relaxed)) as usize;

            // Read raw encrypted bytes directly from socket (no TLS decryption!)
            let mut buf = vec![0u8; current_recv_buff];
            let bytes_read = match conn.read_encrypted_bytes(&mut buf) {
                Ok(n) => n,
                Err(err) => {
                    if !CTRL_C_PRESSED.load(Ordering::Relaxed) {
                        log::error!("Error reading from socket: {err}");
                    }
                    // Connection error, break to create a new connection
                    break;
                }
            };

            if bytes_read == 0 {
                if total_bytes_sank == 0 {
                    log::error!("Cloudflare sent an empty response?");
                }
                // Connection exhausted, break inner loop to make a new request
                break;
            }

            // Count the encrypted bytes we received (wire bytes including TLS overhead)
            total_bytes_sank += bytes_read;
            total_bytes_counter.fetch_add(bytes_read, Ordering::SeqCst);
        }
    }
}

// Spawn a given amount of threads to run a specific test
fn spawn_test_threads<F>(
    threads_to_spawn: u32,
    target_test: Arc<F>,
    bytes_to_request: usize,
    total_bytes_counter: &Arc<AtomicUsize>,
    current_speed: &Arc<AtomicUsize>,
    exit_signal: &Arc<AtomicBool>,
) -> Vec<JoinHandle<()>>
where
    F: Fn(
            usize,
            &Arc<AtomicUsize>,
            &Arc<AtomicUsize>,
            &Arc<AtomicBool>,
        ) -> std::result::Result<(), Box<dyn std::error::Error>>
        + Send
        + Sync
        + 'static,
{
    let mut thread_handles = vec![];

    for i in 0..threads_to_spawn {
        let target_test_clone = Arc::clone(&target_test);
        let total_downloaded_bytes_counter = Arc::clone(&total_bytes_counter.clone());
        let current_down_clone = Arc::clone(&current_speed.clone());
        let exit_signal_clone = Arc::clone(&exit_signal.clone());
        let handle = std::thread::spawn(move || {
            if i > 0 {
                // sleep a little to hit a new cloudflare metal
                // (each metal will throttle to 1 gigabit)
                std::thread::sleep(std::time::Duration::from_millis(
                    (i * NEW_METAL_SLEEP_MILLIS).into(),
                ));
            }

            loop {
                match target_test_clone(
                    bytes_to_request,
                    &total_downloaded_bytes_counter,
                    &current_down_clone,
                    &exit_signal_clone,
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        if !CTRL_C_PRESSED.load(Ordering::Relaxed) {
                            log::error!("Error in download test thread {i}: {e:?}");
                        }
                        return;
                    }
                }

                // exit if we have passed the deadline
                if exit_signal_clone.load(Ordering::Relaxed) {
                    // log::info!("Thread {} exiting...", i);
                    return;
                }
            }
        });
        thread_handles.push(handle);
    }

    thread_handles
}

pub fn run_download_test(config: &UserArgs, results: Arc<Mutex<TestResults>>) -> Vec<usize> {
    let total_downloaded_bytes_counter = Arc::new(AtomicUsize::new(0));
    let exit_signal = Arc::new(AtomicBool::new(false));

    exit_signal.store(false, Ordering::SeqCst);
    let current_down_speed = Arc::new(AtomicUsize::new(0));
    let down_deadline = get_secs_since_unix_epoch()
        + get_test_time(config.test_duration_seconds, config.download_threads);

    let target_test = Arc::new(download_test);
    let down_handles = spawn_test_threads(
        config.download_threads,
        target_test,
        config.bytes_to_download,
        &total_downloaded_bytes_counter,
        &current_down_speed,
        &exit_signal,
    );

    let mut last_bytes_down = 0;
    total_downloaded_bytes_counter.store(0, Ordering::SeqCst);
    let mut down_measurements = vec![];

    // Calculate and log download speed
    loop {
        let bytes_down = total_downloaded_bytes_counter.load(Ordering::Relaxed);
        let bytes_down_diff = bytes_down - last_bytes_down;

        // set current_down
        current_down_speed.store(bytes_down_diff, Ordering::SeqCst);
        down_measurements.push(bytes_down_diff);

        // Update shared results
        if let Ok(mut shared_results) = results.try_lock() {
            shared_results.down_measurements = down_measurements.clone();
        }

        let speed_values = get_appropriate_byte_unit(bytes_down_diff as u64);
        // only log progress if we are before deadline
        if get_secs_since_unix_epoch() < down_deadline {
            log::info!(
                "Download: {bit_speed:>12.*}it/s       ({byte_speed:>10.*}/s)",
                16,
                16,
                byte_speed = speed_values.0,
                bit_speed = speed_values.1
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        last_bytes_down = bytes_down;

        // exit if we have passed the deadline
        if get_secs_since_unix_epoch() > down_deadline {
            exit_signal.store(true, Ordering::SeqCst);
            break;
        }
    }

    log::info!("Waiting for download threads to finish...");
    for handle in down_handles {
        handle.join().expect("Couldn't join download thread");
    }

    // Mark download as completed
    if let Ok(mut shared_results) = results.lock() {
        shared_results.down_measurements = down_measurements.clone();
        shared_results.download_completed = true;
    }

    down_measurements
}

pub fn run_upload_test(config: &UserArgs, results: Arc<Mutex<TestResults>>) -> Vec<usize> {
    let exit_signal = Arc::new(AtomicBool::new(false));
    let total_uploaded_bytes_counter = Arc::new(AtomicUsize::new(0));
    let current_up_speed = Arc::new(AtomicUsize::new(0));
    // re-use exit_signal for upload tests
    exit_signal.store(false, Ordering::SeqCst);

    let up_deadline = get_secs_since_unix_epoch()
        + get_test_time(config.test_duration_seconds, config.upload_threads);

    let target_test = Arc::new(upload_test);
    let up_handles = spawn_test_threads(
        config.upload_threads,
        target_test,
        config.bytes_to_upload,
        &total_uploaded_bytes_counter,
        &current_up_speed,
        &exit_signal,
    );

    let mut last_bytes_up = 0;
    let mut up_measurements = vec![];
    total_uploaded_bytes_counter.store(0, Ordering::SeqCst);

    // Calculate and log upload speed
    loop {
        let bytes_up = total_uploaded_bytes_counter.load(Ordering::Relaxed);

        let bytes_up_diff = bytes_up - last_bytes_up;
        up_measurements.push(bytes_up_diff);

        // Update shared results
        if let Ok(mut shared_results) = results.try_lock() {
            shared_results.up_measurements = up_measurements.clone();
        }

        let speed_values = get_appropriate_byte_unit(bytes_up_diff as u64);

        log::info!(
            "Upload:   {bit_speed:>12.*}it/s       ({byte_speed:>10.*}/s)",
            16,
            16,
            byte_speed = speed_values.0,
            bit_speed = speed_values.1
        );

        std::thread::sleep(std::time::Duration::from_millis(1000));
        last_bytes_up = bytes_up;

        // exit if we have passed the deadline
        if get_secs_since_unix_epoch() > up_deadline {
            exit_signal.store(true, Ordering::SeqCst);
            break;
        }
    }

    // wait for upload threads to finish
    log::info!("Waiting for upload threads to finish...");
    for handle in up_handles {
        handle.join().expect("Couldn't join upload thread");
    }

    // Mark upload as completed
    if let Ok(mut shared_results) = results.lock() {
        shared_results.up_measurements = up_measurements.clone();
        shared_results.upload_completed = true;
    }

    up_measurements
}

pub fn compute_statistics(data: &mut [usize]) -> (f64, f64, usize, usize, usize, usize) {
    if data.is_empty() {
        return (0f64, 0f64, 0, 0, 0, 0);
    }

    data.sort();

    let len = data.len();
    let sum: usize = data.iter().sum();
    let average = sum as f64 / len as f64;

    let median = if len.is_multiple_of(2) {
        (data[len / 2 - 1] + data[len / 2]) as f64 / 2.0
    } else {
        data[len / 2] as f64
    };

    let p90_index = (0.90 * len as f64).ceil() as usize - 1;
    let p99_index = (0.99 * len as f64).ceil() as usize - 1;

    let min = data[0];
    let max = *data.last().unwrap();

    (median, average, data[p90_index], data[p99_index], min, max)
}
