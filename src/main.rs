use clap::Parser;
use nd_300::cli::{FixArgs, Nd300Cli, Nd300Command};
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
    if let Some(title) = cli.title.clone() {
        config = config.with_title(title);
    }
    config = config.with_speed_duration(cli.speed_duration);

    // Subcommand form takes precedence over the legacy action flags.
    // Both forms produce identical behavior — the subcommand is the preferred
    // surface going forward; flags remain so older scripts keep working.
    if let Some(cmd) = cli.command.clone() {
        let exit_code = match cmd {
            Nd300Command::Fix(args) => {
                nd_300::actions::fix::run(&config, args).await
            }
            Nd300Command::Update => nd_300::actions::update::run(&config).await,
            Nd300Command::ClearDns => nd_300::actions::clear_dns::run(&config).await,
            Nd300Command::Uninstall => nd_300::actions::uninstall::run(&config).await,
        };
        std::process::exit(exit_code);
    }

    // Legacy flag form: `nd300 -f`, `nd300 --update`, etc. Identical effects.
    if cli.update {
        let exit_code = nd_300::actions::update::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.uninstall {
        let exit_code = nd_300::actions::uninstall::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.fix {
        let exit_code = nd_300::actions::fix::run(&config, FixArgs::default()).await;
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
