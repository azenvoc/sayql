//! Advisory locks on the store's files, so the app and every `sayql-mcp` process (one per AI
//! app) never interleave a read-modify-write of the same file and lose each other's changes.
//!
//! Each saved file `name` is guarded by its own empty `.<name>.lock` next to it. The lock is
//! never the data file itself: saves rename a new file over the data file, and a lock held on
//! the old one would guard nothing. The lock is the OS's (`flock` on macOS and Linux,
//! `LockFileEx` on Windows) through std's `File::lock`. Each lock opens its own handle, and
//! both platforms make two handles conflict even inside one process, so the same lock keeps
//! threads apart too. The OS drops it when the process exits, however it exits.
//!
//! A thread holds at most one of these locks at a time ([`Held`]), so two of them can't wait
//! on each other: a nested lock is refused, not waited for.

use crate::error::{AppError, AppResult};
use std::cell::Cell;
use std::fs::{File, OpenOptions, TryLockError};
use std::marker::PhantomData;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long to wait for another SayQL process or thread to finish with a file. Saves take
/// milliseconds; this only runs out when a process is stuck while holding the lock.
const WAIT: Duration = Duration::from_secs(10);
/// The longest pause between tries.
const MAX_PAUSE: Duration = Duration::from_millis(20);

thread_local! {
    /// Whether this thread holds a store lock now.
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// A held lock. Closing the handle (on drop) releases it.
pub(super) struct Held {
    _file: File,
    /// Not `Send`: the lock belongs to the thread that took it ([`HOLDING`]).
    _thread: PhantomData<*const ()>,
}

impl Drop for Held {
    fn drop(&mut self) {
        HOLDING.with(|h| h.set(false));
    }
}

/// Whether this thread holds a store lock now.
pub(super) fn holding() -> bool {
    HOLDING.with(Cell::get)
}

/// Takes the lock that guards `name` in `dir`: exclusive for a change, shared for a read that
/// must not see a change halfway (a log's two files). Waits up to [`WAIT`].
pub(super) fn lock(dir: &Path, name: &str, shared: bool) -> AppResult<Held> {
    if holding() {
        return Err(AppError::User(format!(
            "SayQL tried to open its saved file {name} while it was changing another one. This is a bug in SayQL; please report it."
        )));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(format!(".{name}.lock")))?;
    let start = Instant::now();
    let mut pause = Duration::from_millis(1);
    loop {
        let tried = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match tried {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if start.elapsed() < WAIT => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(MAX_PAUSE);
            }
            Err(TryLockError::WouldBlock) => {
                return Err(AppError::User(format!(
                    "SayQL's saved file {name} has been busy for {} seconds: another SayQL window or AI app connection is holding it. Try again; if it stays busy, quit SayQL and the AI apps connected to it, then reopen SayQL.",
                    WAIT.as_secs()
                )));
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    HOLDING.with(|h| h.set(true));
    Ok(Held {
        _file: file,
        _thread: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sayql-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_lock_keeps_other_threads_out_until_it_is_dropped() {
        let dir = temp_dir();
        let held = lock(&dir, "a.json", false).unwrap();
        let other = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(dir.join(".a.json.lock"))
                    .unwrap();
                let busy = matches!(file.try_lock(), Err(TryLockError::WouldBlock));
                let shared_busy = matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock));
                (busy, shared_busy)
            })
        };
        assert_eq!(other.join().unwrap(), (true, true));
        drop(held);
        let again = std::thread::spawn(move || lock(&dir, "a.json", false).is_ok());
        assert!(again.join().unwrap());
    }

    #[test]
    fn shared_locks_allow_each_other() {
        let dir = temp_dir();
        let _held = lock(&dir, "a.jsonl", true).unwrap();
        let other = std::thread::spawn(move || lock(&dir, "a.jsonl", true).is_ok());
        assert!(other.join().unwrap());
    }

    #[test]
    fn a_nested_lock_is_refused_instead_of_waiting_forever() {
        let dir = temp_dir();
        let held = lock(&dir, "a.json", false).unwrap();
        assert!(lock(&dir, "a.json", false).is_err());
        assert!(lock(&dir, "b.json", true).is_err(), "any second lock");
        drop(held);
        assert!(lock(&dir, "b.json", false).is_ok());
    }
}
