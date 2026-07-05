use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STALE_TEMP_FILE_MIN_AGE: Duration = Duration::from_secs(10 * 60);

pub fn write_file_atomically(path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    cleanup_stale_temp_files(path);

    let temp_path = create_temp_file(path, contents)?;
    let result = replace_file(&temp_path, path);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn create_temp_file(path: &Path, contents: &[u8]) -> io::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("powershift");

    for attempt in 0..10 {
        let temp_path = parent.join(format!(
            "{stem}.tmp-{}-{}-{attempt}",
            std::process::id(),
            unique_suffix()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(mut file) => {
                file.write_all(contents)?;
                file.sync_all()?;
                return Ok(temp_path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique temp file",
    ))
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| io::Error::other(error.to_string()))
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)
}

fn cleanup_stale_temp_files(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let prefix = format!("{stem}.tmp-");

    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with(&prefix) && should_remove_temp_file(file_name, &entry.path()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn should_remove_temp_file(file_name: &str, path: &Path) -> bool {
    if is_legacy_temp_file_name(file_name) {
        return true;
    }

    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= STALE_TEMP_FILE_MIN_AGE)
}

fn is_legacy_temp_file_name(file_name: &str) -> bool {
    let Some(suffix) = file_name.split(".tmp-").nth(1) else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_existing_file_without_leaving_temp_files() {
        let path = std::env::temp_dir().join(format!(
            "powershift-atomic-core-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        write_file_atomically(&path, b"first").expect("first write");
        write_file_atomically(&path, b"second").expect("second write");

        assert_eq!(fs::read_to_string(&path).expect("read"), "second");
        let stem = path.file_stem().expect("stem").to_string_lossy();
        assert!(!path
            .parent()
            .expect("parent")
            .read_dir()
            .expect("read dir")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{stem}.tmp-"))));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn cleanup_keeps_fresh_modern_temp_files_from_other_writers() {
        let path = std::env::temp_dir().join(format!(
            "powershift-atomic-fresh-{}.json",
            std::process::id()
        ));
        let stem = path.file_stem().expect("stem").to_string_lossy();
        let sibling_temp = path
            .parent()
            .expect("parent")
            .join(format!("{stem}.tmp-999999-123456789-0"));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&sibling_temp);
        fs::write(&sibling_temp, b"in-flight").expect("seed temp");

        write_file_atomically(&path, b"committed").expect("write");

        assert!(path.exists());
        assert!(sibling_temp.exists());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&sibling_temp);
    }
}
