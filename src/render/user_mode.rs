use crate::config::Config;
use crate::diagnostics::{DiagnosticResults, DiagnosticStatus};
use crate::render::color::{colorize_status, dim};
use crate::render::table::ReportBuilder;

pub fn render(results: &DiagnosticResults, config: &Config) -> String {
    let label_width = 14;
    let data_width = 40;
    let chars = config.box_chars();

    let mut builder = ReportBuilder::new(label_width, data_width, chars)
        .header(config.title(), config.subtitle());

    // Diagnostic summary header
    builder = builder
        .span_row(&format!("  DIAGNOSTIC SUMMARY"))
        .divider();

    // Render each diagnostic category
    builder = render_diagnostic_row(builder, &results.adapters, config);
    builder = render_diagnostic_row(builder, &results.interfaces, config);
    builder = render_diagnostic_row(builder, &results.gateway, config);
    builder = render_diagnostic_row(builder, &results.dns, config);
    builder = render_diagnostic_row(builder, &results.public_ip, config);
    builder = render_diagnostic_row(builder, &results.latency, config);
    builder = render_diagnostic_row(builder, &results.speed, config);
    builder = render_diagnostic_row(builder, &results.ports, config);

    // Overall status
    let (fail_count, warn_count) = count_issues(results);
    let overall = format_overall(fail_count, warn_count, config);

    builder = builder.divider();
    builder = builder.span_row(&format!("  OVERALL: {}", overall));

    if fail_count > 0 {
        builder = builder.span_row(&dim("  Run 'nd300 -f' to attempt automatic fixes", config));
    }

    let mut output = builder.finish();
    output.push('\n');
    output
}

fn render_diagnostic_row(
    builder: ReportBuilder,
    result: &crate::diagnostics::DiagnosticResult,
    config: &Config,
) -> ReportBuilder {
    let icon = config.status_chars(&result.status);
    let colored_icon = colorize_status(icon, &result.status, config);

    let label = format!("{} {}", colored_icon, result.category);

    // Handle multi-line summaries (speed test can have a second line)
    let lines: Vec<&str> = result.summary.split('\n').collect();
    let mut b = builder.row(&label, lines[0]);

    // Render additional lines with empty label
    for line in lines.iter().skip(1) {
        b = b.row("", line);
    }

    b
}

fn count_issues(results: &DiagnosticResults) -> (usize, usize) {
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

    let fails = statuses
        .iter()
        .filter(|s| ***s == DiagnosticStatus::Fail)
        .count();
    let warns = statuses
        .iter()
        .filter(|s| ***s == DiagnosticStatus::Warn)
        .count();

    (fails, warns)
}

fn format_overall(fails: usize, warns: usize, config: &Config) -> String {
    if fails > 0 && warns > 0 {
        let text = format!(
            "{} failure{}, {} warning{}",
            fails,
            if fails > 1 { "s" } else { "" },
            warns,
            if warns > 1 { "s" } else { "" }
        );
        colorize_status(&text, &DiagnosticStatus::Fail, config)
    } else if fails > 0 {
        let text = format!(
            "{} failure{} detected",
            fails,
            if fails > 1 { "s" } else { "" }
        );
        colorize_status(&text, &DiagnosticStatus::Fail, config)
    } else if warns > 0 {
        let text = format!(
            "{} warning{} detected",
            warns,
            if warns > 1 { "s" } else { "" }
        );
        colorize_status(&text, &DiagnosticStatus::Warn, config)
    } else {
        colorize_status("All diagnostics passed", &DiagnosticStatus::Ok, config)
    }
}
