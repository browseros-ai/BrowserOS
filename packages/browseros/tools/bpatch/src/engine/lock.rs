use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::process::Git;

/// Per-checkout lock held while a mutating bpatch operation runs.
#[derive(Debug)]
pub struct CheckoutLock {
    path: PathBuf,
    file: File,
}

impl CheckoutLock {
    /// Acquires `.git/bpatch/lock` without waiting and records this process as holder.
    pub fn acquire(checkout: impl AsRef<Path>) -> Result<Self, LockError> {
        Self::acquire_named(checkout, "lock")
    }

    /// Acquires `.git/bpatch/store.lock` in the store repo without waiting.
    pub fn acquire_store_repo(store_dir: impl AsRef<Path>) -> Result<Self, LockError> {
        Self::acquire_named(store_dir, "store.lock")
    }

    fn acquire_named(repo: impl AsRef<Path>, name: &str) -> Result<Self, LockError> {
        let repo = repo.as_ref();
        let lock_dir = git_private_dir(repo)?.join("bpatch");
        fs::create_dir_all(&lock_dir)?;
        let path = lock_dir.join(name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        match file.try_lock() {
            Ok(()) => {
                file.set_len(0)?;
                file.seek(SeekFrom::Start(0))?;
                write!(
                    file,
                    "pid {} started {}",
                    std::process::id(),
                    current_time_hms()
                )?;
                file.flush()?;
                Ok(Self { path, file })
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                let holder = read_holder(&mut file).unwrap_or_else(|_| "unknown holder".into());
                Err(LockError::Held {
                    reason: format_holder(&holder),
                })
            }
            Err(std::fs::TryLockError::Error(err)) => Err(err.into()),
        }
    }

    /// Returns the git-private lock path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CheckoutLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Failure to acquire or prepare the checkout lock.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another process already holds the lock.
    #[error("{reason}")]
    Held {
        /// Human-readable holder description.
        reason: String,
    },
    /// Git could not resolve the checkout's private directory.
    #[error(transparent)]
    Git(#[from] anyhow::Error),
    /// The lock file could not be created, locked, or written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn git_private_dir(checkout: &Path) -> Result<PathBuf, anyhow::Error> {
    let git = Git::new(checkout);
    let git_dir = PathBuf::from(git.run_str(&["rev-parse", "--git-dir"])?);
    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(checkout.join(git_dir))
    }
}

fn read_holder(file: &mut File) -> std::io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut holder = String::new();
    file.read_to_string(&mut holder)?;
    Ok(holder.trim().to_string())
}

fn format_holder(holder: &str) -> String {
    if holder.is_empty() {
        return "lock held by unknown holder".to_string();
    }
    if let Some(rest) = holder.strip_prefix("pid ")
        && let Some((pid, started)) = rest.split_once(" started ")
    {
        return format!("lock held by pid {pid} (started {started})");
    }
    format!("lock held by {holder}")
}

fn current_time_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3_600,
        (secs / 60) % 60,
        secs % 60
    )
}
