use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct BufferbloatResult {
    pub unloaded_latency_ms: f64,
    pub loaded_latency_ms: Option<f64>,
    pub grade: String,
    pub bloat_ms: Option<f64>,
    pub description: String,
}

pub async fn collect() -> Option<BufferbloatResult> {
    // Measure unloaded latency first
    let unloaded = measure_latency("1.1.1.1", 3).await?;

    // To properly measure loaded latency we'd need to ping during a speed test.
    // Since the speed test already ran, we estimate based on unloaded latency.
    // A proper implementation would measure during the download phase.

    // For now, do a quick loaded test by downloading and pinging simultaneously
    let loaded = measure_loaded_latency(unloaded).await;

    let (grade, description, bloat) = match loaded {
        Some(loaded_ms) => {
            let diff = loaded_ms - unloaded;
            let grade = if diff < 5.0 {
                "A+"
            } else if diff < 30.0 {
                "A"
            } else if diff < 60.0 {
                "B"
            } else if diff < 200.0 {
                "C"
            } else if diff < 400.0 {
                "D"
            } else {
                "F"
            };

            let desc = match grade {
                "A+" | "A" => "Excellent - minimal bufferbloat".to_string(),
                "B" => "Good - minor bufferbloat".to_string(),
                "C" => "Fair - moderate bufferbloat".to_string(),
                "D" => "Poor - significant bufferbloat".to_string(),
                _ => "Severe bufferbloat detected".to_string(),
            };

            (grade.to_string(), desc, Some(diff))
        }
        None => {
            // Can't measure loaded, just report unloaded
            let grade = if unloaded < 20.0 {
                "A"
            } else if unloaded < 50.0 {
                "B"
            } else {
                "C"
            };
            (
                grade.to_string(),
                "Loaded latency not measured (run without --fast for full test)".to_string(),
                None,
            )
        }
    };

    Some(BufferbloatResult {
        unloaded_latency_ms: unloaded,
        loaded_latency_ms: loaded,
        grade,
        bloat_ms: bloat,
        description,
    })
}

async fn measure_latency(host: &str, count: u32) -> Option<f64> {
    #[cfg(windows)]
    let output = {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args(["-n", &count.to_string(), "-w", "2000", host]);
        super::util::run_with_timeout(cmd, super::util::SLOW).await
    };

    #[cfg(unix)]
    let output = {
        let mut cmd = tokio::process::Command::new("ping");
        cmd.args(["-c", &count.to_string(), "-W", "2", host]);
        super::util::run_with_timeout(cmd, super::util::SLOW).await
    };

    let output = output?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut times = Vec::new();

    for line in text.lines() {
        if let Some(pos) = line.find("time=") {
            let after = &line[pos + 5..];
            let num: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(ms) = num.parse::<f64>() {
                times.push(ms);
            }
        } else if let Some(pos) = line.find("time<") {
            let after = &line[pos + 5..];
            let num: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(ms) = num.parse::<f64>() {
                times.push(ms);
            }
        }
    }

    if times.is_empty() {
        None
    } else {
        Some(times.iter().sum::<f64>() / times.len() as f64)
    }
}

async fn measure_loaded_latency(_unloaded: f64) -> Option<f64> {
    // Quick loaded test: download a chunk while pinging
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    // Start a download in the background
    let download = tokio::spawn(async move {
        if let Ok(resp) = client
            .get("https://speed.cloudflare.com/__down?bytes=25000000")
            .send()
            .await
        {
            let _ = resp.bytes().await;
        }
    });

    // Ping during the download
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let loaded = measure_latency("1.1.1.1", 3).await;

    let _ = download.await;

    loaded
}
