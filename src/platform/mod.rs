// Platform abstraction layer
// Individual diagnostics handle their own platform-specific code via #[cfg] attributes.
// This module provides shared utilities for platform detection and common operations.

mod command;
#[cfg(unix)]
pub mod invoking_user;
pub(crate) use command::system_command;
#[cfg(windows)]
pub(crate) use command::system_std_command;

pub fn platform_name() -> &'static str {
    #[cfg(windows)]
    {
        "Windows"
    }

    #[cfg(target_os = "macos")]
    {
        "macOS"
    }

    #[cfg(target_os = "linux")]
    {
        "Linux"
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        "Unknown"
    }
}

pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        is_elevated_windows()
    }

    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
}

#[cfg(windows)]
fn is_elevated_windows() -> bool {
    // Check if running as administrator
    match system_std_command("net").args(["session"]).output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}
