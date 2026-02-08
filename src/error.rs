use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Network diagnostic failed: {message}")]
    Diagnostic { message: String },

    #[error("Platform operation failed: {message}")]
    Platform { message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("DNS error: {message}")]
    Dns { message: String },

    #[error("Configuration error: {message}")]
    Config { message: String },

    #[cfg(windows)]
    #[error("WMI query failed: {0}")]
    Wmi(#[from] wmi::WMIError),
}

impl AppError {
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic {
            message: message.into(),
        }
    }

    pub fn platform(message: impl Into<String>) -> Self {
        Self::Platform {
            message: message.into(),
        }
    }

    pub fn dns(message: impl Into<String>) -> Self {
        Self::Dns {
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }
}
