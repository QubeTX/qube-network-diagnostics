pub mod actions;
pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod platform;
pub mod render;
pub mod speedtest;

pub use config::Config;
pub use error::{AppError, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
