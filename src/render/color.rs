use crate::config::Config;
use crate::diagnostics::DiagnosticStatus;
use owo_colors::OwoColorize;

pub fn colorize_status(text: &str, status: &DiagnosticStatus, config: &Config) -> String {
    if !config.use_colors {
        return text.to_string();
    }

    match status {
        DiagnosticStatus::Ok => text.green().to_string(),
        DiagnosticStatus::Warn => text.yellow().to_string(),
        DiagnosticStatus::Fail => text.red().to_string(),
        DiagnosticStatus::Skip => text.dimmed().to_string(),
    }
}

pub fn green(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.green().to_string()
    } else {
        text.to_string()
    }
}

pub fn yellow(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.yellow().to_string()
    } else {
        text.to_string()
    }
}

pub fn red(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.red().to_string()
    } else {
        text.to_string()
    }
}

pub fn dim(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

pub fn cyan(text: &str, config: &Config) -> String {
    if config.use_colors {
        text.cyan().to_string()
    } else {
        text.to_string()
    }
}
