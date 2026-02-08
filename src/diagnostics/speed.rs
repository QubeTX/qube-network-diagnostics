use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::time::{Duration, Instant};

use super::DiagnosticResult;
use crate::config::Config;

#[derive(Debug, Clone, Serialize)]
pub struct SpeedResult {
    pub download_mbps: f64,
    pub upload_mbps: f64,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub download_duration_s: f64,
    pub upload_duration_s: f64,
    pub loaded_latency_ms: Option<f64>,
    pub server: String,
}

pub async fn check(config: &Config) -> (DiagnosticResult, Option<SpeedResult>) {
    let duration_secs = config.speed_duration;

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(duration_secs + 30))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return (
                DiagnosticResult::fail("Speed", "Failed to create HTTP client"),
                None,
            );
        }
    };

    let total_steps = 100u64;
    let pb = ProgressBar::new(total_steps);
    let template = if config.use_colors {
        "  {spinner:.cyan} Speed test  [{bar:30.cyan/dim}] {pos}% {msg}"
    } else {
        "  {spinner} Speed test  [{bar:30}] {pos}% {msg}"
    };
    pb.set_style(
        ProgressStyle::default_bar()
            .template(template)
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("━╸─"),
    );
    pb.set_message("Download...");

    // Download test
    let dl_duration = Duration::from_secs(duration_secs / 2);
    let (dl_bytes, dl_time) = run_download_test(&client, dl_duration, &pb, 0, 50).await;

    pb.set_position(50);
    pb.set_message("Upload...");

    // Upload test
    let ul_duration = Duration::from_secs(duration_secs / 2);
    let (ul_bytes, ul_time) = run_upload_test(&client, ul_duration, &pb, 50, 50).await;

    pb.set_position(100);
    pb.finish_and_clear();

    let download_mbps = if dl_time > 0.0 {
        (dl_bytes as f64 * 8.0) / (dl_time * 1_000_000.0)
    } else {
        0.0
    };

    let upload_mbps = if ul_time > 0.0 {
        (ul_bytes as f64 * 8.0) / (ul_time * 1_000_000.0)
    } else {
        0.0
    };

    if dl_bytes == 0 && ul_bytes == 0 {
        return (
            DiagnosticResult::fail("Speed", "Speed test could not complete"),
            None,
        );
    }

    let result_data = SpeedResult {
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_time,
        upload_duration_s: ul_time,
        loaded_latency_ms: None,
        server: "speed.cloudflare.com".to_string(),
    };

    let summary = format_speed_summary(download_mbps, upload_mbps);
    let status = determine_speed_status(download_mbps, upload_mbps);

    let result = match status {
        SpeedStatus::Good => DiagnosticResult::ok("Speed", summary),
        SpeedStatus::Warning(note) => {
            DiagnosticResult::warn("Speed", format!("{}\n{}", summary, note))
        }
        SpeedStatus::Poor(note) => {
            DiagnosticResult::warn("Speed", format!("{}\n{}", summary, note))
        }
    };

    (result, Some(result_data))
}

async fn run_download_test(
    client: &reqwest::Client,
    duration: Duration,
    pb: &ProgressBar,
    pb_start: u64,
    pb_range: u64,
) -> (u64, f64) {
    let start = Instant::now();
    let mut total_bytes: u64 = 0;
    let chunk_size = 10_000_000u64; // 10MB chunks

    while start.elapsed() < duration {
        let url = format!(
            "https://speed.cloudflare.com/__down?bytes={}",
            chunk_size
        );

        match client.get(&url).send().await {
            Ok(resp) => {
                match resp.bytes().await {
                    Ok(bytes) => {
                        total_bytes += bytes.len() as u64;
                        let progress = (start.elapsed().as_secs_f64() / duration.as_secs_f64())
                            .min(1.0);
                        pb.set_position(pb_start + (progress * pb_range as f64) as u64);
                    }
                    Err(_) => break,
                }
            }
            Err(_) => break,
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    (total_bytes, elapsed)
}

async fn run_upload_test(
    client: &reqwest::Client,
    duration: Duration,
    pb: &ProgressBar,
    pb_start: u64,
    pb_range: u64,
) -> (u64, f64) {
    let start = Instant::now();
    let mut total_bytes: u64 = 0;
    let chunk_size = 2_000_000usize; // 2MB chunks for upload
    let payload = vec![0u8; chunk_size];

    while start.elapsed() < duration {
        match client
            .post("https://speed.cloudflare.com/__up")
            .body(payload.clone())
            .send()
            .await
        {
            Ok(_) => {
                total_bytes += chunk_size as u64;
                let progress =
                    (start.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
                pb.set_position(pb_start + (progress * pb_range as f64) as u64);
            }
            Err(_) => break,
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    (total_bytes, elapsed)
}

fn format_speed_summary(download: f64, upload: f64) -> String {
    format!(
        "{} down / {} up",
        format_mbps(download),
        format_mbps(upload)
    )
}

fn format_mbps(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.1} Gbps", mbps / 1000.0)
    } else if mbps >= 100.0 {
        format!("{:.0} Mbps", mbps)
    } else if mbps >= 10.0 {
        format!("{:.1} Mbps", mbps)
    } else {
        format!("{:.2} Mbps", mbps)
    }
}

enum SpeedStatus {
    Good,
    Warning(String),
    Poor(String),
}

fn determine_speed_status(download: f64, upload: f64) -> SpeedStatus {
    if download < 5.0 {
        SpeedStatus::Poor("Download too slow for most activities".to_string())
    } else if upload < 1.0 {
        SpeedStatus::Poor("Upload too slow for video calls".to_string())
    } else if download < 25.0 {
        SpeedStatus::Warning("May struggle with HD streaming".to_string())
    } else if upload < 5.0 {
        SpeedStatus::Warning("Upload may be slow for video calls".to_string())
    } else {
        SpeedStatus::Good
    }
}
