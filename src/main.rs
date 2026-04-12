use clap::Parser;
use nd_300::cli::Nd300Cli;
use nd_300::config::{Config, OutputFormat};
use nd_300::diagnostics::{self, DiagnosticResults, DiagnosticStatus};
use nd_300::render;

#[tokio::main]
async fn main() {
    let cli = Nd300Cli::parse();

    #[cfg(windows)]
    enable_utf8_console();

    let mut config = Config::new().with_colors(!cli.no_color);

    if cli.ascii {
        config = config.with_ascii();
    }
    if cli.json {
        config = config.with_json();
    }
    if cli.tech {
        config = config.with_tech_mode();
    }
    if cli.fast {
        config = config.with_skip_speed();
    }
    if cli.verbose {
        config = config.with_verbose();
    }
    if let Some(title) = cli.title {
        config = config.with_title(title);
    }
    config = config.with_speed_duration(cli.speed_duration);

    // Action flags: exit early without running diagnostics
    if cli.update {
        let exit_code = nd_300::actions::update::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.uninstall {
        let exit_code = nd_300::actions::uninstall::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.fix {
        let exit_code = nd_300::actions::fix::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.clear_dns {
        let exit_code = nd_300::actions::clear_dns::run(&config).await;
        std::process::exit(exit_code);
    }

    // Semi-exit-early: exits on failure, falls through to diagnostics on success
    if cli.dns {
        let dns_result = nd_300::actions::dns::run(&config).await;
        if dns_result != 0 {
            std::process::exit(dns_result);
        }
    }

    let results = diagnostics::run_all(&config).await;

    let output = match config.format {
        OutputFormat::Table => {
            if config.is_tech_mode() {
                render::tech_mode::render(&results, &config)
            } else {
                render::user_mode::render(&results, &config)
            }
        }
        OutputFormat::Json => render::json::render(&results, &config),
    };

    print!("{}", output);

    let exit_code = determine_exit_code(&results);
    std::process::exit(exit_code);
}

fn determine_exit_code(results: &DiagnosticResults) -> i32 {
    let statuses = [
        &results.adapters.status,
        &results.interfaces.status,
        &results.gateway.status,
        &results.dns.status,
        &results.public_ip.status,
        &results.latency.status,
        &results.speed.status,
        &results.ports.status,
    ];

    if statuses.iter().any(|s| **s == DiagnosticStatus::Fail) {
        2
    } else if statuses.iter().any(|s| **s == DiagnosticStatus::Warn) {
        1
    } else {
        0
    }
}

#[cfg(windows)]
fn enable_utf8_console() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        unsafe {
            winapi::um::wincon::SetConsoleOutputCP(65001);
        }
    }
}
