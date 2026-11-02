//! An append-only JSON Lines log in the store folder, shared by the AI apps' activity log and
//! your own. Several processes write it at once (the app and one `sayql-mcp` per AI app), so
//! every append, move and clear holds the log's lock (see `super::lock`), and a read holds it
//! shared to see both files from the same moment. Past `max_bytes` the file moves to its `.1`
//! name (replacing the older one), so a log keeps roughly the last two files' worth. Lines
//! that don't parse are skipped when reading.

use crate::error::AppResult;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{BufRead, Write};
use std::path::Path;

pub(super) struct JsonlLog {
    pub file: &'static str,
    /// Where the full file moves.
    pub old: &'static str,
    pub max_bytes: u64,
}

impl JsonlLog {
    pub fn append<T: Serialize>(&self, dir: &Path, entry: &T) -> AppResult<()> {
        let path = dir.join(self.file);
        // Without the lock, two processes could both see a full file, and the second move
        // would put a nearly empty file over the full one at `.1`.
        let _lock = super::lock::lock(dir, self.file, false)?;
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > self.max_bytes) {
            let _ = std::fs::rename(&path, dir.join(self.old));
        }
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?
            .write_all(line.as_bytes())?;
        Ok(())
    }

    /// The newest `limit` entries that `keep` accepts, newest first.
    pub fn read<T: DeserializeOwned>(
        &self,
        dir: &Path,
        keep: impl Fn(&T) -> bool,
        limit: usize,
    ) -> AppResult<Vec<T>> {
        let _lock = super::lock::lock(dir, self.file, true)?;
        let mut all = Vec::new();
        for name in [self.old, self.file] {
            match std::fs::File::open(dir.join(name)) {
                Ok(f) => all.extend(
                    std::io::BufReader::new(f)
                        .lines()
                        .map_while(Result::ok)
                        .filter_map(|l| serde_json::from_str::<T>(&l).ok())
                        .filter(|e| keep(e)),
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        all.reverse();
        all.truncate(limit);
        Ok(all)
    }

    /// Deletes the log, both files.
    pub fn clear(&self, dir: &Path) -> AppResult<()> {
        let _lock = super::lock::lock(dir, self.file, false)?;
        for name in [self.file, self.old] {
            match std::fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

/// Cuts `s` to at most `max` bytes, on a character boundary, ending in `…` when cut.
pub(super) fn cut(s: &mut String, max: usize) {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: JsonlLog = JsonlLog {
        file: "t.jsonl",
        old: "t.1.jsonl",
        max_bytes: 8,
    };

    #[test]
    fn appends_reads_newest_first_rolls_over_and_clears() {
        let dir = std::env::temp_dir().join(format!("sayql-jsonl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..10u32 {
            LOG.append(&dir, &n).unwrap();
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(LOG.file))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let got: Vec<u32> = LOG.read(&dir, |_| true, 100).unwrap();
        assert_eq!(got.first(), Some(&9));
        assert!(got.windows(2).all(|w| w[0] > w[1]), "{got:?}");
        assert!(dir.join(LOG.old).exists());
        let even: Vec<u32> = LOG.read(&dir, |n: &u32| n.is_multiple_of(2), 2).unwrap();
        assert_eq!(even.len(), 2);
        LOG.clear(&dir).unwrap();
        assert!(LOG.read::<u32>(&dir, |_| true, 10).unwrap().is_empty());
    }

    #[test]
    fn cuts_on_a_character_boundary() {
        let mut s = "é".repeat(10);
        cut(&mut s, 5);
        assert_eq!(s, "éé…");
    }
}
