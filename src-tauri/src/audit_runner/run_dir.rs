//! The folder each audit run keeps its files in: the agent's settings, and for the built-in
//! OpenCode its private home (what it read from the sources, query results). It lives in SayQL's
//! own cache folder, never the shared temp folder (on Linux `/tmp` is open to every user, who
//! could read the run's files or swap the folder for one with their own plugin or `rg` in it),
//! and every level is this user's alone ([`RunDir::create`] refuses a folder others can change).

use crate::error::AppResult;
use crate::repos::private_dir;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Runs inside SayQL: removed when the run ends, and any left over (SayQL quit or crashed
/// mid-run) at the next start.
pub const RUNS: &str = "runs";
/// Runs opened in a terminal: the terminal reads their files after SayQL's part is done, so
/// they're removed only once they're [`TERMINAL_KEEP`] old.
pub const TERMINAL: &str = "terminal";
/// How long a terminal run's files are kept.
pub const TERMINAL_KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// One run's private folder, removed when this is dropped (unless [`RunDir::keep`] was called),
/// so every way out of a run, early failures included, cleans up.
#[derive(Debug)]
pub struct RunDir {
    /// Empty once kept.
    path: PathBuf,
}

impl RunDir {
    /// A new folder in `<audit>/<kind>`, `kind` being [`RUNS`] or [`TERMINAL`]. The audit
    /// folder, the kind's folder and the run's folder are readable by this user only, and every
    /// folder above them must be this user's or the system's and not changeable by other users
    /// (a shared folder such as `/tmp` only with its sticky bit).
    pub fn create(audit: &Path, kind: &str) -> AppResult<RunDir> {
        private_dir(audit)?;
        let parent = audit.join(kind);
        private_dir(&parent)?;
        let id: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(12)
            .collect();
        let path = parent.join(id);
        // Not `create_dir_all`: a folder already there wasn't made by this run.
        std::fs::create_dir(&path)?;
        let dir = RunDir { path };
        private_dir(&dir.path)?;
        check_private(&dir.path)?;
        Ok(dir)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Leaves the folder in place when this is dropped (a terminal run reads it later); the
    /// next start removes it once it's [`TERMINAL_KEEP`] old.
    pub fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.path)
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Refuses a run folder another user of this computer could read or replace: the run's own
/// three levels must be this user's with no access for others, and every folder above them
/// this user's or root's, not writable by others unless sticky.
#[cfg(unix)]
fn check_private(run: &Path) -> AppResult<()> {
    use std::os::unix::fs::MetadataExt;
    // The folder was just made by this process, so its owner is this user.
    let me = std::fs::symlink_metadata(run)?.uid();
    let real = run.canonicalize()?;
    for (level, dir) in real.ancestors().enumerate() {
        let m = std::fs::metadata(dir)?;
        let mode = m.mode();
        let ok = if level < 3 {
            m.uid() == me && mode & 0o077 == 0
        } else {
            (m.uid() == me || m.uid() == 0) && (mode & 0o022 == 0 || mode & 0o1000 != 0)
        };
        if !ok {
            return Err(format!(
                "SayQL won't run the audit: other users of this computer could read or change its files in {}. Make that folder yours and not writable by others (chmod go-w), then run the audit again.",
                dir.display()
            )
            .into());
        }
    }
    Ok(())
}

/// Windows keeps each user's app folders private.
#[cfg(not(unix))]
fn check_private(_: &Path) -> AppResult<()> {
    Ok(())
}

/// Removes run folders an earlier start of SayQL left behind: every in-app run made before
/// `started` (none of this start's runs is that old), and terminal runs older than
/// [`TERMINAL_KEEP`].
pub fn clean_leftovers(audit: &Path, started: SystemTime) {
    remove_older(&audit.join(RUNS), started);
    if let Some(cutoff) = started.checked_sub(TERMINAL_KEEP) {
        remove_older(&audit.join(TERMINAL), cutoff);
    }
}

fn remove_older(dir: &Path, cutoff: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        if meta.modified().is_ok_and(|t| t >= cutoff) {
            continue;
        }
        let _ = if meta.is_dir() {
            std::fs::remove_dir_all(e.path())
        } else {
            std::fs::remove_file(e.path())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sayql-rundir-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_run_folder_is_private_and_removed_when_dropped() {
        let root = temp("drop");
        let audit = root.join("audit");
        let run = RunDir::create(&audit, RUNS).unwrap();
        let path = run.path().to_path_buf();
        assert!(path.starts_with(audit.join(RUNS)));
        std::fs::write(path.join("prompt.txt"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for d in [&audit, &audit.join(RUNS), &path] {
                let mode = std::fs::metadata(d).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700, "{}", d.display());
            }
        }
        drop(run);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_early_failure_still_removes_the_run_folder() {
        let root = temp("early");
        let audit = root.join("audit");
        let fail = || -> AppResult<PathBuf> {
            let run = RunDir::create(&audit, RUNS)?;
            let path = run.path().to_path_buf();
            std::fs::write(path.join("mcp.json"), "{}")?;
            Err(format!("{} failed", path.display()).into())
        };
        let err = fail().unwrap_err().to_string();
        let path = PathBuf::from(err.trim_end_matches(" failed"));
        assert!(path.starts_with(audit.join(RUNS)), "{err}");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_kept_run_folder_stays() {
        let root = temp("keep");
        let run = RunDir::create(&root.join("audit"), TERMINAL).unwrap();
        let path = run.keep();
        assert!(path.is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_others_can_change_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("open");
        let open = root.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let audit = open.join("audit");
        let err = RunDir::create(&audit, RUNS).unwrap_err().to_string();
        assert!(err.contains("other users"), "{err}");
        assert!(
            err.contains(&open.canonicalize().unwrap().display().to_string()),
            "{err}"
        );
        // Nothing is left behind.
        let left: Vec<_> = std::fs::read_dir(audit.join(RUNS)).unwrap().collect();
        assert!(left.is_empty());
        // A shared folder with its sticky bit (as /tmp) is fine: others can't rename what's in it.
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(RunDir::create(&audit, RUNS).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn sayqls_own_folders_are_made_private_again() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("readable");
        let audit = root.join("audit");
        let runs = audit.join(RUNS);
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::fs::set_permissions(&runs, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = RunDir::create(&audit, RUNS).unwrap();
        for d in [&audit, &runs] {
            let mode = std::fs::metadata(d).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", d.display());
        }
        drop(run);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn leftover_runs_are_removed_at_the_next_start() {
        let root = temp("leftovers");
        let audit = root.join("audit");
        let left = RunDir::create(&audit, RUNS).unwrap().keep();
        let terminal = RunDir::create(&audit, TERMINAL).unwrap().keep();
        let started = SystemTime::now() + Duration::from_secs(5);
        clean_leftovers(&audit, started);
        // Every in-app run from before the start goes; a terminal run a week younger stays.
        assert!(!left.exists());
        assert!(terminal.exists());
        clean_leftovers(&audit, started + TERMINAL_KEEP);
        assert!(!terminal.exists());
        // A run made after the start is left alone.
        let current = RunDir::create(&audit, RUNS).unwrap();
        clean_leftovers(&audit, SystemTime::now() - Duration::from_secs(60));
        assert!(current.path().exists());
        drop(current);
        let _ = std::fs::remove_dir_all(root);
    }
}
