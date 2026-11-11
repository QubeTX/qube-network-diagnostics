//! Replay recorded counter traces without making any network requests.
use nd_300::speedtest::measurement_v5::{estimate_trace, summarize_traces, MeasurementTrace};
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let traces: Vec<MeasurementTrace> = serde_json::from_str(&input)?;
    let (download, dl) = summarize_traces(&traces, "download");
    let (upload, ul) = summarize_traces(&traces, "upload");
    println!(
        "{}",
        serde_json::json!({
            "estimates": traces.iter().map(estimate_trace).collect::<Vec<_>>(),
            "download": {"estimate": download, "providers": dl},
            "upload": {"estimate": upload, "providers": ul}
        })
    );
    Ok(())
}
