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
    config.speed_deep = cli.deep;
    config.speed_max_bytes = cli.max_bytes;
    config.speed_mlab_consent = cli.accept_mlab;

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
                    Nd300Command::MigrateCleanup(args) => {
                        nd_300::actions::migrate::run(&config, args).await
                    }
                    Nd300Command::InstallTakeover(args) => nd_300::actions::takeover::run(args),
                    Nd300Command::InstallMaintenance(args) => {
                        nd_300::actions::maintenance::run(args)
                    }
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
    // each individual call is bounded. `run_all` enforces a wall-clock cap
    // internally (see `diagnostics::run_all_cap`) and — when the cap fires —
    // returns *partial* results: completed checks are real, unfinished ones
    // carry fabricated `timed_out` Fail rows. The Ctrl-C race keeps manual
    // interruption prompt. On a healthy network none of this is visible.
    let run_all_cap = diagnostics::run_all_cap(&config);
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

        results = diagnostics::run_all(&config, run_all_cap) => results,
    };

    if results.timed_out && !is_json {
        eprintln!(
            "Note: diagnostics hit the {}s wall-clock cap — results below are \
             partial; checks that didn't finish are marked as timed out.",
            run_all_cap.as_secs()
        );
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use nd_300::diagnostics::DiagnosticResult;

    fn all_ok_results() -> DiagnosticResults {
        DiagnosticResults {
            timestamp: "test".to_string(),
            adapters: DiagnosticResult::ok("Adapters", "1 active"),
            interfaces: DiagnosticResult::ok("Network", "1 up"),
            gateway: DiagnosticResult::ok("Gateway", "reachable"),
            dns: DiagnosticResult::ok("DNS", "resolving"),
            public_ip: DiagnosticResult::ok("Internet", "203.0.113.1"),
            latency: DiagnosticResult::ok("Latency", "low"),
            speed: DiagnosticResult::ok("Speed", "fast"),
            ports: DiagnosticResult::ok("Ports", "open"),
            interface_details: None,
            adapter_details: None,
            gateway_details: None,
            dns_details: None,
            public_ip_details: None,
            latency_details: None,
            speed_details: None,
            port_details: None,
            technician: None,
            timed_out: false,
        }
    }

    #[test]
    fn all_ok_exits_0() {
        assert_eq!(determine_exit_code(&all_ok_results()), 0);
    }

    #[test]
    fn warn_exits_1() {
        let mut r = all_ok_results();
        r.latency = DiagnosticResult::warn("Latency", "moderate");
        assert_eq!(determine_exit_code(&r), 1);
    }

    #[test]
    fn any_fail_exits_2() {
        let mut r = all_ok_results();
        r.latency = DiagnosticResult::warn("Latency", "moderate");
        r.gateway = DiagnosticResult::fail("Gateway", "unreachable");
        assert_eq!(determine_exit_code(&r), 2);
    }

    #[test]
    fn skip_is_ignored() {
        let mut r = all_ok_results();
        r.speed = DiagnosticResult::skip("Speed", "skipped");
        assert_eq!(determine_exit_code(&r), 0);
    }

    /// Fabricated timeout rows are Fail — the exit code must stay 2 so
    /// scripts still see a degraded run, even though triage ignores them.
    #[test]
    fn timed_out_rows_still_exit_2() {
        let mut r = all_ok_results();
        r.ports = DiagnosticResult::timed_out_fail("Ports");
        r.timed_out = true;
        assert_eq!(determine_exit_code(&r), 2);
    }
}
