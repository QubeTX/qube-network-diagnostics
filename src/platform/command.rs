//! System utility resolution independent of the caller's PATH and directory.
//!
//! Diagnostics also run inside elevated repair sessions, so they use the same
//! constructors as mutations. User-scoped Cargo commands and already-proven
//! installer/provider paths deliberately keep their owning command setup.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[cfg(unix)]
const SYSTEM_DIRS: [&str; 4] = ["/usr/sbin", "/usr/bin", "/sbin", "/bin"];
#[cfg(unix)]
const SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// Resolve a bare system utility name without consulting process environment.
/// Missing Unix utilities retain an absolute candidate, so spawning fails
/// normally rather than retrying PATH. Invalid names or a failed Windows
/// directory lookup produce an empty program, which cannot be spawned.
pub(crate) fn system_executable(program: &str) -> PathBuf {
    if program.is_empty()
        || !program
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || matches!(program, "." | "..")
    {
        return PathBuf::new();
    }

    #[cfg(windows)]
    {
        let Ok(directory) = windows_system_directory() else {
            return PathBuf::new();
        };
        if program.eq_ignore_ascii_case("powershell")
            || program.eq_ignore_ascii_case("powershell.exe")
        {
            return directory.join("WindowsPowerShell\\v1.0\\powershell.exe");
        }
        let name = if program.to_ascii_lowercase().ends_with(".exe") {
            program.to_string()
        } else {
            format!("{program}.exe")
        };
        directory.join(name)
    }

    #[cfg(unix)]
    {
        resolve_unix_in(program, &SYSTEM_DIRS.map(Path::new))
    }
}

#[cfg(unix)]
fn resolve_unix_in(program: &str, directories: &[&Path]) -> PathBuf {
    directories
        .iter()
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| directories[0].join(program))
}

/// Construct a system utility command, retaining a trusted PATH for scripts
/// such as wg-quick that invoke additional utilities themselves.
/// Absolute paths supplied by the owning installer/provider proof are kept;
/// relative paths containing directory components are never searched.
pub(crate) fn system_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    tokio::process::Command::from(system_std_command(program))
}

pub(crate) fn system_std_command(program: impl AsRef<OsStr>) -> std::process::Command {
    #[cfg(windows)]
    let directory = match windows_system_directory() {
        Ok(directory) => directory,
        // An absolute caller-owned executable must not escape the environment
        // policy when the OS directory needed to establish that policy fails.
        Err(_) => return std::process::Command::new(""),
    };
    let program = program.as_ref();
    let executable = if Path::new(program).is_absolute() {
        PathBuf::from(program)
    } else {
        program.to_str().map(system_executable).unwrap_or_default()
    };
    let mut command = std::process::Command::new(executable);
    // Absolute executable paths and a fixed PATH do not suppress shell startup
    // scripts or imported Bash functions. System scripts must not inherit
    // either channel from the invoking user's shell.
    for variable in [
        "BASH_ENV",
        "ENV",
        "SHELLOPTS",
        "BASHOPTS",
        "CDPATH",
        "GLOBIGNORE",
    ] {
        command.env_remove(variable);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("BASH_FUNC_") {
            command.env_remove(name);
        }
    }
    #[cfg(unix)]
    command.env("PATH", SYSTEM_PATH);
    #[cfg(windows)]
    {
        // -NoProfile still permits automatic module discovery. Restrict it to
        // the native Windows PowerShell modules, independent of PSModulePath,
        // ProgramFiles, SystemRoot and the invoking user's module directory.
        command.env(
            "PSModulePath",
            directory.join("WindowsPowerShell\\v1.0\\Modules"),
        );
        let mut directories = vec![directory.clone()];
        if let Some(windows) = directory.parent() {
            directories.push(windows.to_path_buf());
        }
        if let Ok(path) = std::env::join_paths(directories) {
            command.env("PATH", path);
        }
    }
    command
}

#[cfg(windows)]
pub(super) fn windows_system_directory() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::sysinfoapi::GetSystemDirectoryW;

    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: the buffer is writable for the exact capacity supplied to Win32.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(format!(
            "Could not resolve the trusted Windows system directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length as usize >= buffer.len() {
        return Err("Windows system directory exceeded the trusted path buffer".to_string());
    }
    let directory = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]));
    if !directory.is_absolute() {
        return Err("Windows returned a non-absolute system directory".to_string());
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_system_names_cannot_escape_or_execute() {
        for program in [
            "",
            ".",
            "..",
            "../sh",
            "/bin/sh",
            "C:\\evil.exe",
            "a/b",
            "a\\b",
        ] {
            assert!(system_executable(program).as_os_str().is_empty());
            if !Path::new(program).is_absolute() {
                assert!(system_std_command(program).status().is_err());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_missing_tool_retains_absolute_trusted_candidate() {
        let directory = Path::new("/nd300-nonexistent-system-directory");
        let executable = resolve_unix_in("missing-utility", &[directory]);
        assert_eq!(executable, directory.join("missing-utility"));
        assert!(executable.is_absolute());
    }

    // Test environment poisoning in a separate process; never change global
    // environment while the rest of the parallel Rust test suite is running.
    #[test]
    fn hostile_search_environment_is_ignored() {
        let root = std::env::temp_dir().join(format!("nd300-command-path-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // An executable copy under a missing utility name would successfully
        // run the harness if any of our spawn paths fell back to PATH/cwd.
        let fake_name = if cfg!(windows) {
            "nd300-untrusted-missing-tool.exe"
        } else {
            "nd300-untrusted-missing-tool"
        };
        std::fs::copy(std::env::current_exe().unwrap(), root.join(fake_name)).unwrap();
        let existing_name = if cfg!(windows) { "cmd.exe" } else { "sh" };
        std::fs::copy(std::env::current_exe().unwrap(), root.join(existing_name)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::command::tests::hostile_search_child",
                "--nocapture",
            ])
            .env("ND300_COMMAND_PATH_CHILD", "1")
            .env("PATH", &root)
            .env("SystemRoot", &root)
            .env("WINDIR", &root)
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(status.success());
    }

    #[test]
    fn hostile_search_child() {
        if std::env::var_os("ND300_COMMAND_PATH_CHILD").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use crate::actions::fix::cmd::{run_cmd, run_cmd_capture, run_owned_mutation};
            use std::time::Duration;
            let budget = Duration::from_secs(5);
            let missing = "nd300-untrusted-missing-tool";
            // Positive control: this same inherited search environment can
            // launch the planted executable through the old bare constructor.
            let untrusted = std::process::Command::new(missing)
                .arg("--nd300-hostile-tool")
                .output()
                .expect("bare program lookup should reach the planted executable");
            assert!(!untrusted.status.success());
            assert!(String::from_utf8_lossy(&untrusted.stderr).contains("nd300-hostile-tool"));
            let missing_command = || {
                let mut command = system_command(missing);
                // If a regression searches PATH, the copied test executable
                // must exit with an argument error, never run its whole suite.
                command.arg("--nd300-hostile-tool");
                command
            };
            assert!(system_executable(missing).is_absolute());
            assert!(run_cmd(missing_command(), budget).await.is_err());
            let captured = run_cmd_capture(missing_command(), budget).await;
            assert!(!captured.ok);
            assert!(captured.exit_code.is_none());
            assert!(run_owned_mutation(missing_command(), budget).await.is_err());
            assert!(
                crate::diagnostics::util::run_with_timeout(missing_command(), budget)
                    .await
                    .is_none()
            );

            #[cfg(windows)]
            let command = {
                let executable = system_executable("powershell");
                assert!(executable.ends_with("WindowsPowerShell\\v1.0\\powershell.exe"));
                assert!(!executable.starts_with(std::env::current_dir().unwrap()));
                let mut command = system_command("cmd");
                command.args(["/D", "/C", "exit 0"]);
                command
            };
            #[cfg(unix)]
            let command = {
                let mut command = system_command("sh");
                command.args(["-c", "exit 0"]);
                command
            };
            assert!(run_cmd(command, budget).await.unwrap().status.success());
        });
    }

    #[test]
    fn hostile_shell_startup_environment_is_ignored() {
        let root =
            std::env::temp_dir().join(format!("nd300-command-startup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let startup = root.join("startup.sh");
        std::fs::write(&startup, "printf 'nd300-startup-loaded\\n'\n").unwrap();
        #[cfg(windows)]
        let modules = {
            let modules = root.join("modules");
            let module = modules.join("Microsoft.PowerShell.Management");
            std::fs::create_dir_all(&module).unwrap();
            // Only discovery is exercised: no service command is invoked.
            std::fs::write(
                module.join("Microsoft.PowerShell.Management.psm1"),
                "function Restart-Service { 'nd300-untrusted-module' }; Export-ModuleMember -Function Restart-Service\n",
            )
            .unwrap();
            modules
        };
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "platform::command::tests::hostile_shell_startup_child",
                "--nocapture",
            ])
            .env("ND300_COMMAND_STARTUP_CHILD", &root)
            .env("BASH_ENV", startup.to_string_lossy().replace('\\', "/"))
            .env("ENV", &startup)
            .env("SHELLOPTS", "xtrace")
            .env("BASHOPTS", "expand_aliases")
            .env(
                "BASH_FUNC_nd300_hostile_function%%",
                "() { printf hostile; }",
            );
        #[cfg(windows)]
        child.env(
            "PSModulePath",
            std::env::join_paths([
                modules,
                windows_system_directory()
                    .unwrap()
                    .join("WindowsPowerShell\\v1.0\\Modules"),
            ])
            .unwrap(),
        );
        let status = child.status().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(status.success());
    }

    #[test]
    fn hostile_shell_startup_child() {
        let Some(root) = std::env::var_os("ND300_COMMAND_STARTUP_CHILD").map(PathBuf::from) else {
            return;
        };
        // On native Unix hosts Bash is an OS shell. Git Bash provides the
        // same harmless startup regression on Windows developer machines;
        // Windows CI without Git Bash still exercises PowerShell below.
        #[cfg(unix)]
        let bash = Some(PathBuf::from("/bin/bash"));
        #[cfg(windows)]
        let bash = [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files\Git\usr\bin\bash.exe",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file());
        if let Some(bash) = bash {
            let args = [
                "--noprofile",
                "--norc",
                "-c",
                "type -t nd300_hostile_function || :; printf 'expected\\n'",
            ];
            let untrusted = std::process::Command::new(&bash)
                .args(args)
                .output()
                .unwrap();
            assert!(untrusted.status.success());
            let stdout = String::from_utf8_lossy(&untrusted.stdout);
            assert!(stdout.contains("nd300-startup-loaded"), "{stdout}");
            assert!(stdout.contains("function"), "{stdout}");
            let secured = system_std_command(&bash).args(args).output().unwrap();
            assert!(secured.status.success(), "{:?}", secured.stderr);
            assert_eq!(String::from_utf8_lossy(&secured.stdout).trim(), "expected");
            assert!(!String::from_utf8_lossy(&secured.stderr).contains("++"));
        }

        #[cfg(windows)]
        {
            let args = [
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Command Restart-Service | ForEach-Object { $_.CommandType.ToString(); $_.Module.Path }",
            ];
            let powershell = system_executable("powershell");
            let untrusted = std::process::Command::new(&powershell)
                .args(args)
                .output()
                .unwrap();
            assert!(untrusted.status.success(), "{:?}", untrusted.stderr);
            let stdout = String::from_utf8_lossy(&untrusted.stdout);
            assert!(
                stdout.lines().any(|line| line.trim() == "Function"),
                "{stdout}"
            );
            assert!(
                stdout.contains(&root.file_name().unwrap().to_string_lossy().to_string()),
                "{stdout}"
            );
            let secured = system_std_command(&powershell).args(args).output().unwrap();
            assert!(secured.status.success(), "{:?}", secured.stderr);
            let stdout = String::from_utf8_lossy(&secured.stdout);
            assert!(
                stdout.lines().any(|line| line.trim() == "Cmdlet"),
                "{stdout}"
            );
            let modules = windows_system_directory()
                .unwrap()
                .join("WindowsPowerShell\\v1.0\\Modules");
            assert!(
                stdout
                    .lines()
                    .any(|line| Path::new(line.trim()).starts_with(&modules)),
                "{stdout}"
            );
        }
        #[cfg(unix)]
        let _ = root;
    }
}
