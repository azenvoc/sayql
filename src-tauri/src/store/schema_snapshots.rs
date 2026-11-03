//! The last schema seen per database, kept as fingerprints only (no data), so the next load
//! can say what changed: a column renamed rather than dropped, a table gone.

use super::Store;
use crate::error::AppResult;
use crate::schema::anchors::Snapshot;
use std::collections::BTreeMap;

const FILE: &str = "schema-snapshots.json";

fn key(connection_id: &str, database: Option<&str>) -> String {
    format!("{connection_id}\u{1f}{}", database.unwrap_or_default())
}

impl Store {
    pub fn schema_snapshot(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> AppResult<Option<Snapshot>> {
        let all: BTreeMap<String, Snapshot> = self.read(FILE)?;
        Ok(all.get(&key(connection_id, database)).cloned())
    }

    pub fn save_schema_snapshot(
        &self,
        connection_id: &str,
        database: Option<&str>,
        snapshot: Snapshot,
    ) -> AppResult<()> {
        self.update(FILE, |all: &mut BTreeMap<String, Snapshot>| {
            all.insert(key(connection_id, database), snapshot);
            Ok(())
        })
    }

    /// Forgets a deleted connection's snapshots.
    pub fn forget_schema_snapshots(&self, connection_id: &str) -> AppResult<()> {
        let prefix = format!("{connection_id}\u{1f}");
        self.update(FILE, |all: &mut BTreeMap<String, Snapshot>| {
            all.retain(|k, _| !k.starts_with(&prefix));
            Ok(())
        })
    }
}
