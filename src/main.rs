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
    if cli.yes {
        config = config.with_auto_confirm_medium_risk();
    }
    if let Some(title) = cli.title.clone() {
        config = config.with_title(title);
    }
    config = config.with_speed_duration(cli.speed_duration);

    // Subcommand form takes precedence over the legacy action flags.
    // Both forms produce identical behavior — the subcommand is the preferred
    // surface going forward; flags remain so older scripts keep working.
    //
    // `dns` is special: like the legacy `-d`/`--dns` flag it is *semi*-exit-early
    // (exit on failure, fall through to diagnostics on success), so it must NOT
    // go through the terminal subcommand block below. It instead sets the same
    // `run_dns` intent the flag does, making `nd300 dns` ≡ `nd300 --dns`.
    let mut run_dns = cli.dns;
    if let Some(cmd) = cli.command.clone() {
        match cmd {
            Nd300Command::Dns => run_dns = true,
            terminal => {
                let exit_code = match terminal {
                    Nd300Command::Fix(args) => nd_300::actions::fix::run(&config, args).await,
                    Nd300Command::Update => nd_300::actions::update::run(&config).await,
                    Nd300Command::ClearDns => nd_300::actions::clear_dns::run(&config).await,
                    Nd300Command::Uninstall => nd_300::actions::uninstall::run(&config).await,
                    // `Dns` is handled above (semi-exit-early), never reaches here.
                    Nd300Command::Dns => unreachable!(),
                };
                std::process::exit(exit_code);
            }
        }
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

    // Semi-exit-early: exits on failure, falls through to diagnostics on success.
    // Driven by either the `-d`/`--dns` flag or the `nd300 dns` subcommand.
    if run_dns {
        let dns_result = nd_300::actions::dns::run(&config).await;
        if dns_result != 0 {
            std::process::exit(dns_result);
        }
    }

    // Safety net: diagnostics shell out and resolve hostnames, which on a
    // badly-broken network could still take longer than is useful even though
    // each individual call is now bounded. Race the whole run against an
    // overall wall-clock cap and a Ctrl-C handler so the tool always returns
    // promptly with a clear message instead of appearing to hang. On a healthy
    // network the diagnostics finish first and this is invisible.
    //
    // The cap must scale with `--speed-duration`: the diagnostic speed test runs
    // the CF + NDT7 providers sequentially, each doing a download AND an upload
    // for `speed_duration` seconds per direction, so the legitimate speed-test
    // floor is ~`4 * speed_duration` (2 providers × 2 directions). A fixed 90s
    // would falsely truncate a deliberately long speed test (e.g.
    // `--speed-duration 60`) and misreport it as a "severely degraded network."
    // 30s of headroom covers discovery/latency probes and the non-speed
    // diagnostics; a 90s floor keeps the default/`--fast` behavior unchanged.
    const RUN_ALL_CAP_FLOOR: std::time::Duration = std::time::Duration::from_secs(90);
    const RUN_ALL_CAP_HEADROOM: u64 = 30;
    let run_all_cap = if config.skip_speed {
        RUN_ALL_CAP_FLOOR
    } else {
        std::time::Duration::from_secs(4 * config.speed_duration + RUN_ALL_CAP_HEADROOM)
            .max(RUN_ALL_CAP_FLOOR)
    };
    let is_json = matches!(config.format, OutputFormat::Json);

    let results = tokio::select! {
        biased;

        _ = tokio::signal::ctrl_c() => {
            if is_json {
                println!("{{\"error\":\"interrupted\",\"interrupted\":true}}");
            } else {
                eprintln!("Interrupted.");
            }
            std::process::exit(130);
        }

        outcome = tokio::time::timeout(run_all_cap, diagnostics::run_all(&config)) => {
            match outcome {
                Ok(results) => results,
                Err(_) => {
                    if is_json {
                        println!("{{\"error\":\"timeout\",\"timed_out\":true}}");
                    } else {
                        eprintln!(
                            "Diagnostics timed out after {}s — your network appears to be \
                             severely degraded or unreachable. Check your connection and \
                             try again.",
                            run_all_cap.as_secs()
                        );
                    }
                    std::process::exit(2);
                }
            }
        }
    };

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
