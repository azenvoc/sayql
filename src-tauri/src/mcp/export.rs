//! `export_csv`: a read-only query's rows saved to a new CSV file in Downloads, where the
//! connection lets AI apps see all rows.

use super::shape::{failed, withhold};
use super::*;

/// Most rows `export_csv` writes.
pub(super) const EXPORT_ROWS: usize = 100_000;
/// How long an export may run before SayQL stops it.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(600);
pub(super) const EXPORT_OFF: &str = "The user hasn't allowed AI apps to see all rows on this connection, so export_csv is off \
here. Use open_query to put the query in a SayQL tab: the user can run it and save the rows from there.";

impl SayqlServer {
    /// Saves a read-only query's rows to a CSV in Downloads, where the connection allows all rows.
    pub(super) async fn export(
        &self,
        p: ExportParams,
        log: &mut Activity,
    ) -> Result<String, String> {
        let target = Target::new(p.connection_id, p.database);
        // The AI app could read the file back, so exporting needs the same permission as seeing rows.
        let access = self.access(&target.connection_id)?;
        log.may_see = Some(access.apps);
        if access.apps != AppResults::All {
            return Err(refused(log, "export needs all rows", EXPORT_OFF.into()));
        }
        let pool = self
            .state
            .target_pool(&target)
            .await
            .map_err(|e| e.to_string())?;
        // The same checks as run_query: the AI app could read the file back.
        let guard = self.checked(&pool, &p.sql, &access.never_send, log).await?;
        let mut never = Masker::new(&access.never_send);
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .ok_or("Could not find the Downloads folder.")?;
        let mut r = gate::run(
            &pool,
            &p.sql,
            EXPORT_ROWS,
            EXPORT_TIMEOUT,
            &CancellationToken::new(),
        )
        .await
        .map_err(|e| failed(log, access.apps, &guard, e))?;
        never.learn_aliases(&p.sql);
        never.withhold_positions(&guard.withheld);
        withhold(&never, &r.columns, &mut r.rows);
        log.rows = Some(r.rows.len());
        let (path, mut file) = new_export_file(&dir, p.file_name.as_deref())?;
        file.write_all(to_csv(&r.columns, &r.rows).as_bytes())
            .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(json!({
            "path": path,
            "columns": r.columns,
            "row_count": r.rows.len(),
            "truncated": r.truncated,
            "note": if r.truncated { "Stopped at 100,000 rows; narrow the query for the rest." } else { "All rows saved." },
        })
        .to_string())
    }
}

/// RFC 4180 CSV, like "Copy as CSV" in the app. NULL becomes an empty field.
pub(super) fn to_csv(columns: &[String], rows: &[Vec<Value>]) -> String {
    pub(super) fn field(s: &str) -> String {
        if s.contains(['"', ',', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let cell = |v: &Value| match v {
        Value::Null => String::new(),
        Value::String(s) => field(s),
        other => field(&other.to_string()),
    };
    let mut out = columns
        .iter()
        .map(|c| field(c))
        .collect::<Vec<_>>()
        .join(",");
    for row in rows {
        out.push('\n');
        out.push_str(&row.iter().map(cell).collect::<Vec<_>>().join(","));
    }
    out.push('\n');
    out
}

/// A new file in `dir` named after `name` (letters, digits, spaces, `-`, `_`, `.`), never an existing one.
pub(super) fn new_export_file(
    dir: &Path,
    name: Option<&str>,
) -> Result<(PathBuf, std::fs::File), String> {
    let clean: String = name
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.".contains(*c))
        .take(80)
        .collect();
    let stem = clean
        .trim()
        .trim_end_matches(".csv")
        .trim_matches('.')
        .trim();
    let stem = if stem.is_empty() {
        "sayql-export"
    } else {
        stem
    };
    for n in 1..1000 {
        let path = dir.join(if n == 1 {
            format!("{stem}.csv")
        } else {
            format!("{stem} ({n}).csv")
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not create {}: {e}", path.display())),
        }
    }
    Err(format!("Too many files named {stem} in {}.", dir.display()))
}
