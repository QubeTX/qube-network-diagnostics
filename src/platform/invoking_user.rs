//! Resolve the real desktop user when ND300 is running through `sudo`.
//!
//! Network repairs need root, but reports and user-scoped tools must continue
//! to use the invoking user's home directory and ownership.  We deliberately
//! trust only the numeric `SUDO_UID`/`SUDO_GID` pair and confirm the UID via
//! the system passwd database; `SUDO_USER`, `HOME`, and `USER` are not treated
//! as identity authorities.

use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The account that launched ND300, even when the process is elevated by
/// `sudo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokingUser {
    uid: libc::uid_t,
    gid: libc::gid_t,
    username: String,
    home: PathBuf,
    effective_uid: libc::uid_t,
}

impl InvokingUser {
    /// Resolve the invoking account from numeric sudo metadata and the passwd
    /// database. Without a valid sudo UID, this resolves the effective user.
    pub fn detect() -> io::Result<Self> {
        // SAFETY: geteuid has no preconditions and does not dereference
        // pointers.
        let effective_uid = unsafe { libc::geteuid() };

        // A present-but-invalid SUDO_UID must fail closed. Silently treating a
        // malformed value as "no sudo" would redirect reports and Cargo work
        // into root's home, recreating the /var/root bug this context prevents.
        let sudo_uid = if effective_uid == 0 {
            match std::env::var("SUDO_UID") {
                Ok(value) => Some(value.parse::<libc::uid_t>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "SUDO_UID is not numeric")
                })?),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SUDO_UID is not valid Unicode",
                    ));
                }
            }
        } else {
            None
        };
        let uid = sudo_uid.unwrap_or(effective_uid);
        let passwd = passwd_entry(uid)?;

        let gid = if sudo_uid.is_some() {
            let sudo_gid = match std::env::var("SUDO_GID") {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SUDO_GID is not valid Unicode",
                    ));
                }
            };
            validated_sudo_gid(sudo_gid.as_deref(), passwd.gid)?
        } else {
            passwd.gid
        };

        if !passwd.home.is_absolute() || passwd.home.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invoking user's passwd entry has no absolute home directory",
            ));
        }

        Ok(Self {
            uid,
            gid,
            username: passwd.username,
            home: passwd.home,
            effective_uid,
        })
    }

    pub fn uid(&self) -> libc::uid_t {
        self.uid
    }

    pub fn gid(&self) -> libc::gid_t {
        self.gid
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Cargo home for the invoking account. An explicit `CARGO_HOME` is
    /// honored only when ND300 is not crossing from root to another user;
    /// sudo-inherited root environment variables must never redirect a user
    /// update into `/var/root`.
    pub fn cargo_home(&self) -> PathBuf {
        if !self.is_different_from_effective_user() {
            if let Some(path) = std::env::var_os("CARGO_HOME").map(PathBuf::from) {
                if path.is_absolute() {
                    return path;
                }
            }
        }
        self.home.join(".cargo")
    }

    pub fn is_different_from_effective_user(&self) -> bool {
        self.uid != self.effective_uid
    }

    /// Run a user-scoped child as the invoking account rather than root.
    pub fn configure_command(&self, command: &mut tokio::process::Command) {
        command.env("HOME", &self.home);
        command.env("USER", &self.username);
        command.env("LOGNAME", &self.username);
        command.env("CARGO_HOME", self.cargo_home());
        if self.is_different_from_effective_user() {
            // Tokio forwards these to std::process::Command. Rust's Unix child
            // setup calls setgid, then clears supplementary groups with
            // setgroups(0, NULL), then calls setuid. Do not add a pre_exec
            // initgroups callback here: NSS/initgroups is not async-signal-safe
            // after fork in this multi-threaded process.
            command.uid(self.uid);
            command.gid(self.gid);
        }
    }

    /// Write a private file through a same-directory temporary file, then
    /// atomically rename it into place. The file is mode 0600 and is owned by
    /// the invoking user even when ND300 itself is root.
    pub fn write_private_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "output path has no parent")
        })?;
        let directory = match parent.strip_prefix(&self.home) {
            Ok(_) => self.open_directory_beneath_home(parent, true)?,
            Err(_) if !self.is_different_from_effective_user() => {
                // A direct, non-elevated invocation may intentionally use an
                // absolute XDG_CONFIG_HOME outside the passwd home. There is no
                // cross-user privilege boundary in that case, but still pin the
                // final directory with O_NOFOLLOW before touching a filename.
                self.ensure_directory(parent)?;
                open_directory_nofollow(parent, self.uid)?
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "elevated output path is outside the invoking user's home",
                ));
            }
        };
        let target_name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "output path has no filename")
        })?;
        let target_name = component_cstring(target_name)?;

        let mut temp_name = None;
        let mut temp_file = None;
        for attempt in 0..100_u32 {
            let candidate = format!(".nd300-{}.{}.tmp", std::process::id(), attempt);
            let candidate_c = CString::new(candidate.as_bytes()).expect("fixed temporary name");
            match openat_create_new(directory.as_raw_fd(), &candidate_c, 0o600) {
                Ok(file) => {
                    temp_name = Some(candidate_c);
                    temp_file = Some(file);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        let temp_name = temp_name.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a private temporary report file",
            )
        })?;
        let mut file = temp_file.expect("temporary path and file are created together");

        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            if self.is_different_from_effective_user() {
                fchown(&file, self.uid, self.gid)?;
                file.sync_all()?;
            }
            drop(file);
            renameat_same_directory(directory.as_raw_fd(), &temp_name, &target_name)?;
            directory.sync_all()?;
            Ok(())
        })();

        if result.is_err() {
            let _ = unlinkat_file(directory.as_raw_fd(), &temp_name);
        }
        result
    }

    /// Ensure a user-scoped directory exists and belongs to the invoking user.
    pub fn ensure_directory(&self, path: &Path) -> io::Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "output parent is a symlink or is not a directory",
                    ));
                }
                // Never chown a pre-existing user-controlled directory.
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let parent = path.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "directory has no parent")
                })?;
                self.ensure_directory(parent)?;
                std::fs::create_dir(path)?;
                let directory = File::open(path)?;
                if self.is_different_from_effective_user() {
                    fchown(&directory, self.uid, self.gid)?;
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Validate and create a report directory strictly beneath the passwd home
    /// without following a symlink out of that tree.
    pub fn ensure_directory_beneath_home(&self, path: &Path) -> io::Result<()> {
        self.open_directory_beneath_home(path, true).map(drop)
    }

    /// Open a stable descriptor for a directory beneath the passwd home.
    /// Every component is traversed relative to the previously opened
    /// descriptor with O_NOFOLLOW, so a user-controlled symlink swap cannot
    /// redirect an elevated report or updater operation outside that home.
    pub(crate) fn open_directory_beneath_home(
        &self,
        path: &Path,
        create: bool,
    ) -> io::Result<File> {
        let relative = path.strip_prefix(&self.home).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output directory is outside the invoking user's home",
            )
        })?;
        if relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output directory contains unsafe path components",
            ));
        }

        let canonical_home = std::fs::canonicalize(&self.home)?;
        let mut directory = open_directory_nofollow(&canonical_home, self.uid)?;
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "output directory contains unsafe path components",
                ));
            };
            let name = component_cstring(name)?;
            directory = match openat_directory(directory.as_raw_fd(), &name, self.uid) {
                Ok(child) => child,
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    mkdirat_owned(directory.as_raw_fd(), &name, self.uid, self.gid)?;
                    openat_directory(directory.as_raw_fd(), &name, self.uid)?
                }
                Err(error) => return Err(error),
            };
        }
        Ok(directory)
    }
}

fn component_cstring(name: &OsStr) -> io::Result<CString> {
    if name.as_bytes().is_empty() || name.as_bytes().contains(&b'/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path component is empty or contains a separator",
        ));
    }
    CString::new(name.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "path component contains a NUL byte",
        )
    })
}

fn open_directory_nofollow(path: &Path, expected_uid: libc::uid_t) -> io::Result<File> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path contains a NUL byte",
        )
    })?;
    // SAFETY: path is a live NUL-terminated string. The returned descriptor is
    // immediately owned by File on success.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(normalize_nofollow_error(io::Error::last_os_error()));
    }
    // SAFETY: open returned a new owned descriptor.
    let directory = unsafe { File::from_raw_fd(fd) };
    verify_directory_owner(&directory, expected_uid)?;
    Ok(directory)
}

fn openat_directory(parent: RawFd, name: &CStr, expected_uid: libc::uid_t) -> io::Result<File> {
    let directory = openat_directory_unchecked(parent, name)?;
    verify_directory_owner(&directory, expected_uid)?;
    Ok(directory)
}

fn openat_directory_unchecked(parent: RawFd, name: &CStr) -> io::Result<File> {
    // SAFETY: parent is a live directory fd and name is NUL-terminated.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(normalize_nofollow_error(io::Error::last_os_error()));
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn verify_directory_owner(directory: &File, expected_uid: libc::uid_t) -> io::Result<()> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage and the fd is live.
    if unsafe { libc::fstat(directory.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fstat succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened path is not a directory",
        ));
    }
    if stat.st_uid != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "user-scoped directory is owned by uid {}, expected uid {}",
                stat.st_uid, expected_uid
            ),
        ));
    }
    Ok(())
}

fn mkdirat_owned(parent: RawFd, name: &CStr, uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    // SAFETY: parent is a live directory fd and name is NUL-terminated.
    let created = if unsafe { libc::mkdirat(parent, name.as_ptr(), 0o700) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        false
    } else {
        true
    };
    if created {
        // An elevated process creates the component as root. Pin it without
        // following links, transfer ownership through the descriptor, then
        // verify the final type and owner before traversing it.
        let child = openat_directory_unchecked(parent, name)?;
        child.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        fchown(&child, uid, gid)?;
        verify_directory_owner(&child, uid)
    } else {
        openat_directory(parent, name, uid).map(drop)
    }
}

fn openat_create_new(parent: RawFd, name: &CStr, mode: libc::mode_t) -> io::Result<File> {
    // SAFETY: parent is a live directory fd and name is NUL-terminated.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            libc::c_uint::from(mode),
        )
    };
    if fd < 0 {
        Err(normalize_nofollow_error(io::Error::last_os_error()))
    } else {
        // SAFETY: openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn renameat_same_directory(parent: RawFd, old: &CStr, new: &CStr) -> io::Result<()> {
    // SAFETY: both names are NUL-terminated and parent is a live directory fd.
    if unsafe { libc::renameat(parent, old.as_ptr(), parent, new.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn unlinkat_file(parent: RawFd, name: &CStr) -> io::Result<()> {
    // SAFETY: name is NUL-terminated and parent is a live directory fd.
    if unsafe { libc::unlinkat(parent, name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn normalize_nofollow_error(error: io::Error) -> io::Error {
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOTDIR)
    ) {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path contains a symbolic link or non-directory component",
        )
    } else {
        error
    }
}

fn validated_sudo_gid(value: Option<&str>, passwd_gid: libc::gid_t) -> io::Result<libc::gid_t> {
    let Some(value) = value else {
        return Ok(passwd_gid);
    };
    let supplied = value
        .parse::<libc::gid_t>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "SUDO_GID is not numeric"))?;
    if supplied != passwd_gid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "SUDO_GID {} does not match passwd primary gid {}",
                supplied, passwd_gid
            ),
        ));
    }
    Ok(passwd_gid)
}

#[derive(Debug)]
struct PasswdEntry {
    gid: libc::gid_t,
    username: String,
    home: PathBuf,
}

fn passwd_entry(uid: libc::uid_t) -> io::Result<PasswdEntry> {
    // 16 KiB is comfortably above typical _SC_GETPW_R_SIZE_MAX values while
    // remaining bounded if the platform reports no recommendation.
    let recommended = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let size = if recommended > 0 {
        usize::try_from(recommended)
            .unwrap_or(16 * 1024)
            .clamp(1024, 64 * 1024)
    } else {
        16 * 1024
    };
    let mut buffer = vec![0_u8; size];
    let mut passwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();

    // SAFETY: passwd points to writable storage, buffer is valid for its
    // length, and result is an out-pointer. We copy strings before the buffer
    // leaves scope.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            passwd.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    if result.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no passwd entry for uid {uid}"),
        ));
    }

    // SAFETY: getpwuid_r succeeded and returned the same initialized passwd
    // storage through `result`; its string pointers remain valid in buffer.
    let passwd = unsafe { passwd.assume_init() };
    if passwd.pw_name.is_null() || passwd.pw_dir.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "passwd entry is missing a username or home directory",
        ));
    }
    let username = unsafe { CStr::from_ptr(passwd.pw_name) }
        .to_string_lossy()
        .into_owned();
    let home = PathBuf::from(
        unsafe { CStr::from_ptr(passwd.pw_dir) }
            .to_string_lossy()
            .into_owned(),
    );

    Ok(PasswdEntry {
        gid: passwd.pw_gid,
        username,
        home,
    })
}

fn fchown(file: &File, uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    // SAFETY: the fd belongs to the live File, and uid/gid are plain values.
    let status = unsafe { libc::fchown(file.as_raw_fd(), uid, gid) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_account_resolves_to_an_absolute_home() {
        let user = InvokingUser::detect().expect("current passwd entry");
        assert!(user.home().is_absolute());
        assert!(!user.username().is_empty());
    }

    #[test]
    fn private_atomic_write_uses_mode_0600() {
        let user = InvokingUser::detect().expect("current passwd entry");
        let root =
            std::env::temp_dir().join(format!("nd300-invoking-user-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("report.md");

        user.write_private_atomic(&path, b"private\n").unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"private\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sudo_gid_must_match_the_passwd_primary_gid() {
        assert_eq!(validated_sudo_gid(None, 501).unwrap(), 501);
        assert_eq!(validated_sudo_gid(Some("501"), 501).unwrap(), 501);
        assert!(validated_sudo_gid(Some("20"), 501).is_err());
        assert!(validated_sudo_gid(Some("not-a-number"), 501).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_write_refuses_a_symlink_parent() {
        use std::os::unix::fs::symlink;

        let user = InvokingUser::detect().expect("current passwd entry");
        let root =
            std::env::temp_dir().join(format!("nd300-symlink-parent-test-{}", std::process::id()));
        let target = root.join("target");
        let link = root.join("linked-parent");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&target).unwrap();
        symlink(&target, &link).unwrap();

        let error = user
            .write_private_atomic(&link.join("report.md"), b"must not write\n")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!target.join("report.md").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn home_relative_write_refuses_an_intermediate_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "nd300-home-walk-symlink-test-{}",
            std::process::id()
        ));
        let home = root.join("home");
        let outside = root.join("outside");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, home.join("Downloads")).unwrap();

        let mut user = InvokingUser::detect().expect("current passwd entry");
        user.home = home.clone();
        let error = user
            .write_private_atomic(&home.join("Downloads/report.md"), b"private\n")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!outside.join("report.md").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn private_atomic_write_replaces_a_target_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "nd300-report-target-symlink-test-{}",
            std::process::id()
        ));
        let home = root.join("home");
        let downloads = home.join("Downloads");
        let victim = root.join("victim.txt");
        let report = downloads.join("report.md");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(&victim, b"do not modify\n").unwrap();
        symlink(&victim, &report).unwrap();

        let mut user = InvokingUser::detect().expect("current passwd entry");
        user.home = home;
        user.write_private_atomic(&report, b"new report\n").unwrap();

        assert_eq!(std::fs::read(&victim).unwrap(), b"do not modify\n");
        assert_eq!(std::fs::read(&report).unwrap(), b"new report\n");
        assert!(!std::fs::symlink_metadata(&report)
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = std::fs::remove_dir_all(root);
    }
}
