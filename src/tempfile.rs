//! Fichier temporaire anonyme, sans dépendance.
//!
//! * Unix : créé en `0600` puis supprimé immédiatement (il disparaît même après un `kill -9`).
//! * Windows : `FILE_FLAG_DELETE_ON_CLOSE`.
//! * Autres plateformes : suppression au `Drop`.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempFile {
    pub(crate) file: File,
    #[cfg(not(any(unix, windows)))]
    path: PathBuf,
}

#[cfg(not(any(unix, windows)))]
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl TempFile {
    pub(crate) fn create(dir: Option<&Path>) -> io::Result<TempFile> {
        let dir: PathBuf = match dir {
            Some(d) => d.to_path_buf(),
            None => std::env::temp_dir(),
        };
        let mut last = None;
        for _ in 0..16 {
            let path = dir.join(unique_name());
            match open_new(&path) {
                Ok(file) => return finish(file, path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::AlreadyExists, "aucun nom de fichier temporaire libre")
        }))
    }
}

fn unique_name() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("spillvec-{}-{}-{:x}.tmp", process::id(), n, nanos)
}

#[cfg(unix)]
fn open_new(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(unix)]
fn finish(file: File, path: PathBuf) -> io::Result<TempFile> {
    std::fs::remove_file(&path)?;
    Ok(TempFile { file })
}

#[cfg(windows)]
fn open_new(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const DELETE: u32 = 0x0001_0000;
    const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x0000_0100;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .attributes(FILE_ATTRIBUTE_TEMPORARY)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
}

#[cfg(windows)]
fn finish(file: File, _path: PathBuf) -> io::Result<TempFile> {
    Ok(TempFile { file })
}

#[cfg(not(any(unix, windows)))]
fn open_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create_new(true).open(path)
}

#[cfg(not(any(unix, windows)))]
fn finish(file: File, path: PathBuf) -> io::Result<TempFile> {
    Ok(TempFile { file, path })
}
