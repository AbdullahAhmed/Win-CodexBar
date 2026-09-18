use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Replace a staged sibling file with the destination using the platform's
/// replacement semantics. Windows `rename` does not replace an existing
/// file, so use `MoveFileExW` with replace and write-through flags there.
pub fn replace_staged(staged: &Path, destination: &Path) -> io::Result<()> {
    // Callers may use a secure writer that does not expose its file handle.
    // Sync the staged bytes here so every replacement has the same durability
    // boundary before the destination is changed.
    std::fs::OpenOptions::new()
        .write(true)
        .open(staged)?
        .sync_all()?;

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        use windows::core::PCWSTR;

        let staged: Vec<u16> = staged.as_os_str().encode_wide().chain([0]).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: both buffers are NUL-terminated and remain alive for the call.
        unsafe {
            MoveFileExW(
                PCWSTR(staged.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(|error| io::Error::other(error.to_string()))
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(staged, destination)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::File::open(parent)?.sync_all()
    }
}

/// Atomic file write: temp sibling + fsync + rename. On failure, truncate the
/// owned temp sibling instead of deleting it so callers never need destructive cleanup.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!(
            "output directory does not exist: {} (it is not created)",
            parent.display()
        );
    }
    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = PathBuf::from(temp_name);
    let result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_staged(&temp, path)?;
        Ok(())
    })();
    if result.is_err()
        && let Ok(file) = std::fs::OpenOptions::new().write(true).open(&temp)
    {
        let _truncated = file.set_len(0);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::replace_staged;

    #[test]
    fn replacement_updates_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        let destination = dir.path().join("destination");
        std::fs::write(&staged, b"new").unwrap();
        std::fs::write(&destination, b"old").unwrap();

        replace_staged(&staged, &destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!staged.exists());
    }
}
