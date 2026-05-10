use crate::config::{Config, OutputFormat};
use crate::render::color;
use crate::render::progress::create_spinner;

use super::{fail_icon, flush_dns_platform, print_elevation_hint, success_icon};

pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json().await;
    }

    let spinner = create_spinner("Flushing DNS cache...");

    let result = flush_dns_platform().await;

    spinner.finish_and_clear();

    match result {
        Ok(msg) => {
            println!(
                "  {} {}",
                color::green(success_icon(config), config),
                color::green("DNS cache flushed successfully", config),
            );
            if config.verbose && !msg.is_empty() {
                println!("    {}", color::dim(&msg, config));
            }
            0
        }
        Err(msg) => {
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red("Failed to flush DNS cache", config),
            );
            if !msg.is_empty() {
                println!("    {}", color::dim(&msg, config));
            }
            if !crate::platform::is_elevated() {
                print_elevation_hint(config);
            }
            2
        }
    }
}

async fn run_json() -> i32 {
    let (success, message) = match flush_dns_platform().await {
        Ok(msg) => (true, msg),
        Err(msg) => (false, msg),
    };

    let output = serde_json::json!({
        "action": "clear_dns",
        "dns_flush": {
            "success": success,
            "message": message,
        }
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );

    if success {
        0
    } else {
        2
    }
}
