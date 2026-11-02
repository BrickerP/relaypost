use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt, fs::PermissionsExt};

pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn check_id(s: &str) -> Result<()> {
    ensure!(valid_digest(s), "invalid parcel ID or fingerprint");
    Ok(())
}

pub(crate) fn no_symlink_components(path: &Path) -> Result<()> {
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir),
            "parent path components are forbidden"
        );
        current.push(component.as_os_str());
        let meta = fs::symlink_metadata(&current)
            .with_context(|| format!("cannot inspect {}", current.display()))?;
        ensure!(
            !meta.file_type().is_symlink(),
            "symbolic link refused: {}",
            current.display()
        );
    }
    Ok(())
}

pub(crate) fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "not a real directory: {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        fs::create_dir_all(path)?;
    }
    directory(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// A user-selected root may have OS aliases among its ancestors (for example
/// /var on macOS). Resolve those once; never resolve links inside a parcel,
/// published batch or private application subtree.
pub(crate) fn selected_directory(path: &Path) -> Result<std::path::PathBuf> {
    directory(path)?;
    Ok(fs::canonicalize(path)?)
}

pub(crate) fn output_directory(path: &Path) -> Result<std::path::PathBuf> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let canonical = selected_directory(path)?;
    if let Some(parent) = canonical.parent() {
        sync_dir(parent)?;
    }
    Ok(canonical)
}

pub(crate) fn selected_file(path: &Path) -> Result<std::path::PathBuf> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "selected file must be a regular file, not a symbolic link"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent)?;
    Ok(parent.join(path.file_name().context("selected file has no name")?))
}

pub(crate) fn open_read(path: &Path) -> Result<File> {
    no_symlink_components(path)?;
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "not a regular file: {}",
        path.display()
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    // A path can change after inspection. Never wait for a FIFO writer before
    // the opened descriptor's regular-file check below can reject it.
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "not a regular file: {}",
        path.display()
    );
    Ok(file)
}

pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut file = open_read(path)?;
    ensure!(
        file.metadata()?.len() <= max,
        "file exceeds resource limit: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    (&mut file).take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "file grew beyond resource limit");
    Ok(bytes)
}

pub(crate) fn create_new(path: &Path) -> Result<File> {
    let parent = path.parent().context("file needs a parent directory")?;
    no_symlink_components(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    Ok(options.open(path)?)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    directory(path)?;
    File::open(path)?
        .sync_all()
        .with_context(|| format!("directory sync unsupported: {}", path.display()))?;
    Ok(())
}

pub(crate) fn sync_tree(path: &Path) -> Result<()> {
    directory(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir() {
            sync_tree(&entry.path())?;
        } else {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "special file in private staging"
            );
            open_read(&entry.path())?.sync_all()?;
        }
    }
    sync_dir(path)
}

/// The OS performs both the existence check and rename in one operation.
/// There is deliberately no check-then-rename or copy fallback.
pub(crate) fn publish_noreplace(source: &Path, destination: &Path) -> Result<()> {
    no_symlink_components(source)?;
    no_symlink_components(destination.parent().context("destination has no parent")?)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let from = CString::new(source.as_os_str().as_bytes())?;
        let to = CString::new(destination.as_os_str().as_bytes())?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) as libc::c_long
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context(
                "atomic no-replace publication failed (target must support exclusive rename)",
            );
        }
        sync_dir(destination.parent().context("destination has no parent")?)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("atomic no-replace publication is supported only on Linux and macOS")
}

pub(crate) fn lock(home: &Path) -> Result<File> {
    let path = home.join(".lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}

pub(crate) fn available_space(path: &Path, needed: u64) -> Result<()> {
    let available = fs2::available_space(path)?;
    ensure!(
        available >= needed,
        "insufficient space: need {needed} bytes, available {available}"
    );
    Ok(())
}

pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn sqlite_timeout() -> Duration {
    Duration::from_secs(10)
}

pub(crate) fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // All protocol structs use deny_unknown_fields. Serde's derived struct
    // visitors reject duplicate fields (including nested records).
    serde_json::from_slice(bytes).context("invalid or ambiguous JSON record")
}

pub(crate) fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).context("record is not UTF-8")
}

pub(crate) fn assert_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(_) => anyhow::bail!("destination already exists: {}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_read_accepts_regular_files() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let path = root.join("regular");
        fs::write(&path, b"synthetic regular file")?;
        assert_eq!(read_bounded(&path, 64)?, b"synthetic regular file");
        assert!(open_read(&root).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn open_read_refuses_symbolic_links_and_devices() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        fs::write(root.join("regular"), b"synthetic regular file")?;
        let link = root.join("link");
        std::os::unix::fs::symlink(root.join("regular"), &link)?;
        assert!(open_read(&link).is_err());
        let ancestor = root.join("ancestor");
        std::os::unix::fs::symlink(&root, &ancestor)?;
        assert!(open_read(&ancestor.join("regular")).is_err());
        assert!(open_read(Path::new("/dev/null"))
            .unwrap_err()
            .to_string()
            .contains("not a regular file"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn open_read_refuses_fifo_without_waiting_for_a_writer() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let fifo = root.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes())?;
        let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        ensure!(result == 0, "{}", std::io::Error::last_os_error());
        let (sender, receiver) = std::sync::mpsc::channel();
        let path = fifo.clone();
        let reader = std::thread::spawn(move || {
            let _ = sender.send(open_read(&path).map(|_| ()));
        });
        let result = receiver.recv_timeout(Duration::from_secs(5));
        if result.is_err() {
            // Release a regressed blocking open so a failed test does not
            // leave its worker waiting indefinitely.
            let _writer = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo)?;
            reader.join().expect("FIFO reader panicked");
            anyhow::bail!("opening a FIFO waited for a writer");
        }
        reader.join().expect("FIFO reader panicked");
        assert!(result?
            .unwrap_err()
            .to_string()
            .contains("not a regular file"));
        Ok(())
    }

    #[test]
    fn atomic_publication_never_replaces_existing_directory() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let existing = root.join("committed");
        private_dir(&existing)?;
        write_new(&existing.join("original"), b"synthetic original")?;
        let staging = root.join("staging");
        private_dir(&staging)?;
        write_new(&staging.join("replacement"), b"synthetic replacement")?;
        assert!(publish_noreplace(&staging, &existing).is_err());
        assert_eq!(fs::read(existing.join("original"))?, b"synthetic original");
        assert!(!existing.join("replacement").exists());
        assert!(staging.join("replacement").exists());
        let fresh = root.join("fresh");
        publish_noreplace(&staging, &fresh)?;
        assert_eq!(
            fs::read(fresh.join("replacement"))?,
            b"synthetic replacement"
        );
        Ok(())
    }
}
