
use crate::{TestResults, locations, table};
use crate::speed_test::{compute_statistics, get_appropriate_byte_unit_rate, get_current_timestamp, get_download_server_http_latency, get_download_server_info, get_our_ip_address_country};


pub fn print_test_preamble() {
    println!("{:<32} {}", "Start:", get_current_timestamp());

    let our_country = get_our_ip_address_country().expect("Couldn't get our country");
    let our_country_full = locations::CCA2_TO_COUNTRY_NAME.get(&our_country as &str);
    let latency = get_download_server_http_latency().expect("Couldn't get server latency");
    let headers = get_download_server_info().expect("Couldn't get download server info");

    let unknown_colo = &"???".to_owned();
    let unknown_colo_info = &("UNKNOWN", "UNKNOWN");
    let cf_colo = headers.get("cf-meta-colo").unwrap_or(unknown_colo);
    let colo_info = locations::IATA_TO_CITY_COUNTRY
        .get(cf_colo as &str)
        .unwrap_or(unknown_colo_info);

    println!(
        "{:<32} {}",
        "Your Location:",
        our_country_full.unwrap_or(&"UNKNOWN")
    );
    println!(
        "{:<32} {} - {}, {}",
        "Server Location:",
        cf_colo,
        colo_info.0,
        locations::CCA2_TO_COUNTRY_NAME
            .get(colo_info.1)
            .unwrap_or(&"UNKNOWN")
    );

    println!("{:<32} {:.2}ms\n", "Latency (HTTP):", latency.as_millis());
}

pub fn print_results_table(results: &TestResults) {
    let mut down_measurements = results.down_measurements.clone();
    let mut up_measurements = results.up_measurements.clone();

    let (download_median, download_avg, download_p90, _, _, _) =
        compute_statistics(&mut down_measurements);
    let (upload_median, upload_avg, upload_p90, _, _, _) = compute_statistics(&mut up_measurements);

    let mut rows = vec![vec![
        "".to_string(),
        "Median".to_string(),
        "Average".to_string(),
        "90th pctile".to_string(),
    ]];

    // Populate rows based on computed statistics
    if results.download_completed || !results.down_measurements.is_empty() {
        rows.push(vec![
            "DOWN".to_string(),
            get_appropriate_byte_unit_rate(download_median as u64).1,
            get_appropriate_byte_unit_rate(download_avg as u64).1,
            get_appropriate_byte_unit_rate(download_p90 as u64).1,
        ]);
    }

    if results.upload_completed || !results.up_measurements.is_empty() {
        rows.push(vec![
            "UP".to_string(),
            get_appropriate_byte_unit_rate(upload_median as u64).1,
            get_appropriate_byte_unit_rate(upload_avg as u64).1,
            get_appropriate_byte_unit_rate(upload_p90 as u64).1,
        ]);
    }

    let table = table::format_ascii_table(rows);
    print!("\n{}\n{}\n", get_current_timestamp(), table);
}