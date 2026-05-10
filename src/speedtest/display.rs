use crate::config::BoxChars;
use crate::render::table::ReportBuilder;
use crate::VERSION;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;

use super::{format_bytes, format_mbps, SpeedTestResult};

/// Manages live CLI progress display for the SpeedQX binary.
pub struct SpeedQXDisplay {
    use_ascii: bool,
    use_colors: bool,
    json_mode: bool,
}

impl SpeedQXDisplay {
    pub fn new(use_ascii: bool, use_colors: bool, json_mode: bool) -> Self {
        Self {
            use_ascii,
            use_colors,
            json_mode,
        }
    }

    /// Print the SpeedQX header banner.
    pub fn print_header(&self) {
        if self.json_mode {
            return;
        }
        println!();
        if self.use_colors {
            println!(
                "  {} - Internet Speed Test",
                format!("SpeedQX v{}", VERSION).cyan().bold()
            );
            println!("  {}", "QubeTX Developer Tools".dimmed());
        } else {
            println!("  SpeedQX v{} - Internet Speed Test", VERSION);
            println!("  QubeTX Developer Tools");
        }
        println!();
    }

    /// Create an indicatif spinner for a phase step.
    pub fn create_spinner(&self, step: u32, total: u32, msg: &str) -> ProgressBar {
        if self.json_mode {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new_spinner();
        let template = format!("  [{{spinner}}] [{}/{}] {{msg}}", step, total);
        pb.set_style(
            ProgressStyle::default_spinner()
                .template(&template)
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        pb
    }

    /// Create an indicatif progress bar for download/upload phases.
    pub fn create_progress_bar(&self, step: u32, total: u32, msg: &str) -> ProgressBar {
        if self.json_mode {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new(100);
        let template = format!("  [{{bar:20.cyan/dim}}] [{}/{}] {{msg}}", step, total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&template)
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("##-"),
        );
        pb.set_message(msg.to_string());
        pb
    }

    /// Print a persistent completion line for a finished step.
    pub fn finish_step(&self, step: u32, total: u32, msg: &str) {
        if self.json_mode {
            return;
        }
        let check = if self.use_ascii { "[OK]" } else { "\u{2713}" };
        if self.use_colors {
            println!("  [{}/{}] {} {}", step, total, check.green(), msg,);
        } else {
            println!("  [{}/{}] {} {}", step, total, check, msg);
        }
    }
}

/// Render the final results table for SpeedQX output.
pub fn render_results(result: &SpeedTestResult, use_ascii: bool, use_colors: bool) -> String {
    let chars = if use_ascii {
        BoxChars::ascii()
    } else {
        BoxChars::unicode()
    };

    let label_width = 14;
    let data_width = 27;

    let single_provider = result.providers.len() == 1;

    if single_provider {
        return render_single_provider(result, chars, label_width, data_width, use_colors);
    }

    render_multi_provider(result, chars, label_width, data_width, use_colors)
}

fn render_multi_provider(
    result: &SpeedTestResult,
    chars: BoxChars,
    label_width: usize,
    data_width: usize,
    _use_colors: bool,
) -> String {
    let mut builder = ReportBuilder::new(label_width, data_width, chars);

    // Top border + title
    builder = builder.full_top_border().span_row(&format!(
        "  {:^width$}",
        "SPEEDQX RESULTS",
        width = label_width + data_width + 3
    ));

    builder = builder.section_header("Averaged Results");

    // Ping
    if let Some(ping) = result.ping_ms {
        builder = builder.row("Ping", &format!("{:.1} ms", ping));
    }

    // Jitter
    if let Some(jitter) = result.jitter_ms {
        builder = builder.row("Jitter", &format!("{:.1} ms", jitter));
    }

    // Download / Upload
    builder = builder.row(
        "Download",
        &format!("{} (avg)", format_mbps(result.download_mbps)),
    );
    builder = builder.row(
        "Upload",
        &format!("{} (avg)", format_mbps(result.upload_mbps)),
    );

    // Packet loss
    if let Some(loss) = result.packet_loss_pct {
        builder = builder.row("Packet Loss", &format!("{}%", loss));
    }

    // Duration
    builder = builder.row("Duration", &format!("{:.1}s", result.duration_s));

    // Stability metrics
    if let Some(ref stability) = result.stability {
        let dl_label = if stability.download_stable {
            "Stable"
        } else {
            "Variable"
        };
        let ul_label = if stability.upload_stable {
            "Stable"
        } else {
            "Variable"
        };
        builder = builder.row(
            "Stability",
            &format!(
                "DL: {} (CV {:.0}%) / UL: {} (CV {:.0}%)",
                dl_label,
                stability.download_cv * 100.0,
                ul_label,
                stability.upload_cv * 100.0,
            ),
        );
    }

    // Provider divergence warning
    if let Some(ref div) = result.provider_divergence {
        if div.significant {
            builder = builder.row(
                "Divergence",
                &format!(
                    "DL {:.0}% / UL {:.0}% (significant)",
                    div.download * 100.0,
                    div.upload * 100.0,
                ),
            );
        }
    }

    // Per-provider sections
    for provider in &result.providers {
        builder = render_provider_section(builder, provider);
    }

    let mut output = builder.finish();
    output.push('\n');
    output
}

fn render_single_provider(
    result: &SpeedTestResult,
    chars: BoxChars,
    label_width: usize,
    data_width: usize,
    _use_colors: bool,
) -> String {
    let mut builder = ReportBuilder::new(label_width, data_width, chars);

    builder = builder.full_top_border().span_row(&format!(
        "  {:^width$}",
        "SPEEDQX RESULTS",
        width = label_width + data_width + 3
    ));

    if let Some(provider) = result.providers.first() {
        builder = builder.section_header(&provider.provider);

        // Server
        builder = builder.row("Server", &provider.server);

        // Location
        if let Some(ref loc) = provider.location {
            builder = builder.row("Location", loc);
        }

        // Ping
        if let Some(ping) = provider.ping_ms {
            builder = builder.row("Ping", &format!("{:.1} ms", ping));
        }

        // Jitter
        if let Some(jitter) = provider.jitter_ms {
            builder = builder.row("Jitter", &format!("{:.1} ms", jitter));
        }

        // Download / Upload
        if let Some(dl) = provider.download_mbps {
            builder = builder.row("Download", &format_mbps(dl));
        }
        if let Some(ul) = provider.upload_mbps {
            builder = builder.row("Upload", &format_mbps(ul));
        }

        // Data transferred
        builder = builder.row("DL Data", &format_bytes(provider.download_bytes));
        builder = builder.row("UL Data", &format_bytes(provider.upload_bytes));

        // Packet loss
        if let Some(loss) = provider.packet_loss_pct {
            builder = builder.row("Packet Loss", &format!("{}%", loss));
        }

        // Duration
        builder = builder.row("Duration", &format!("{:.1}s", result.duration_s));
    }

    let mut output = builder.finish();
    output.push('\n');
    output
}

fn render_provider_section(
    builder: ReportBuilder,
    provider: &super::ProviderResult,
) -> ReportBuilder {
    let mut b = builder.section_header(&provider.provider);

    // Error case
    if let Some(ref err) = provider.error {
        b = b.row("Error", err);
        return b;
    }

    // Server
    b = b.row("Server", &provider.server);

    // Location
    if let Some(ref loc) = provider.location {
        b = b.row("Location", loc);
    }

    // Ping
    if let Some(ping) = provider.ping_ms {
        b = b.row("Ping", &format!("{:.1} ms", ping));
    }

    // Jitter
    if let Some(jitter) = provider.jitter_ms {
        b = b.row("Jitter", &format!("{:.1} ms", jitter));
    }

    // Download / Upload
    if let Some(dl) = provider.download_mbps {
        b = b.row("Download", &format_mbps(dl));
    }
    if let Some(ul) = provider.upload_mbps {
        b = b.row("Upload", &format_mbps(ul));
    }

    // Data transferred
    b = b.row("DL Data", &format_bytes(provider.download_bytes));
    b = b.row("UL Data", &format_bytes(provider.upload_bytes));

    b
}
