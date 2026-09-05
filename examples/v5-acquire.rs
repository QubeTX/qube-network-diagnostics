//! Local controlled-reference harness. Uses the production acquisition adapter.
use nd_300::speedtest::{
    acquisition_v5::{client, transfer, RunBudget},
    measurement_v5::estimate_trace,
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let endpoint = args.get(1).ok_or("Expected a loopback endpoint")?;
    let url = url::Url::parse(endpoint)?;
    let isolated_reference = std::env::var("SPEEDQX_ISOLATED_REFERENCE").as_deref() == Ok("1")
        && url.host_str() == Some("192.0.2.2")
        && url.scheme() == "http";
    if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) && !isolated_reference {
        return Err(
            "Harness accepts loopback or the opt-in isolated TEST-NET-1 fixture only".into(),
        );
    }
    let direction = args.get(2).map(String::as_str).unwrap_or("download");
    if !matches!(direction, "download" | "upload") {
        return Err("Expected download or upload".into());
    }
    let duration =
        Duration::from_millis(args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(10000));
    let budget = RunBudget::new(
        500_000_000,
        Duration::from_secs(90),
        Arc::new(AtomicBool::new(false)),
    );
    let client = client()?;
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64()
        * 1000.0;
    let trace = transfer(
        "cloudflare",
        endpoint,
        direction,
        duration,
        &client,
        &budget,
        |_| {},
    )
    .await;
    println!(
        "{}",
        serde_json::json!({"estimate": estimate_trace(&trace), "trace": trace, "budgetBytes": budget.used(), "startedAt": started_at})
    );
    Ok(())
}
