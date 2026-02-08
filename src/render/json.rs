use crate::config::Config;
use crate::diagnostics::DiagnosticResults;

pub fn render(results: &DiagnosticResults, _config: &Config) -> String {
    match serde_json::to_string_pretty(results) {
        Ok(json) => format!("{}\n", json),
        Err(e) => {
            let fallback = serde_json::json!({
                "error": format!("Failed to serialize results: {}", e)
            });
            format!("{}\n", fallback)
        }
    }
}
