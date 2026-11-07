//! Which crabsim processes are still alive, so a run whose process ended before it did can be
//! found. Every catalogue that uses the results store holds an exclusive lock on a file of its own,
//! `.crabsim/processes/<id>.lock`, for as long as it lives, and each run records that id. The
//! operating system releases the lock however the process ends, even when it crashes or is
//! killed, so if another process can take the lock, the run's process is gone.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The folder of lock files inside `.crabsim`.
pub const PROCESSES_FOLDER: &str = "processes";

/// This process's lock, held until dropped.
pub(crate) struct ProcessLock {
    id: String,
    path: PathBuf,
    file: Option<File>,
}

impl ProcessLock {
    /// Takes a lock of this process's own in `crabsim_folder`.
    pub fn acquire(crabsim_folder: &Path) -> std::io::Result<ProcessLock> {
        let folder = crabsim_folder.join(PROCESSES_FOLDER);
        std::fs::create_dir_all(&folder)?;
        let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        let id = format!("{}-{started}", std::process::id());
        let path = folder.join(format!("{id}.lock"));
        let file = File::create_new(&path)?;
        file.try_lock().map_err(|error| match error {
            TryLockError::Error(error) => error,
            TryLockError::WouldBlock => std::io::Error::other("a new lock file is already locked"),
        })?;
        Ok(ProcessLock { id, path, file: Some(file) })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        // Closing the file releases the lock, and on Windows a file can only go once it's closed.
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether the process with lock id `id` is still alive. A process whose lock file is gone, or
/// whose lock can be taken, has ended, and its lock file is removed. If the lock can't be tested,
/// the process counts as alive, so a run is never marked interrupted by mistake.
pub(crate) fn is_alive(crabsim_folder: &Path, id: &str) -> bool {
    let path = crabsim_folder.join(PROCESSES_FOLDER).join(format!("{id}.lock"));
    let Ok(file) = File::options().read(true).write(true).open(&path) else {
        return path.exists();
    };
    match file.try_lock() {
        Ok(()) => {
            drop(file);
            let _ = std::fs::remove_file(&path);
            false
        }
        Err(_) => true,
    }
}

/// Removes the lock files of processes that have ended, such as ones whose runs were forgotten.
pub(crate) fn tidy(crabsim_folder: &Path, own: &str) {
    let Ok(entries) = std::fs::read_dir(crabsim_folder.join(PROCESSES_FOLDER)) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".lock").filter(|id| *id != own) {
            is_alive(crabsim_folder, id);
        }
    }
}
