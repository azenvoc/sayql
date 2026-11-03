//! The audit: instructions for an AI app working in the person's code (Claude Code, Cursor)
//! to teach SayQL about a database. The app reads the schema, its usage figures and the saved
//! notes, checks them against the code, and suggests notes with `suggest_note`, including
//! corrections and removals of notes that look outdated. Served as the MCP prompt `audit`
//! (`/mcp__sayql__audit` in Claude Code) and copied from the app for AI apps without prompts.

pub mod coding_agents;
pub mod knowledge;
pub mod opencode;
pub mod process;
pub mod repos;
pub mod run;
pub mod runner;
pub mod sources;

use crate::db::ConnConfig;
use crate::store::Store;

/// The MCP prompt's name.
pub const PROMPT: &str = "audit";
/// What the prompt does, in prompts/list.
pub const DESCRIPTION: &str = "Teach SayQL about a database: read its schema, usage figures and saved notes, \
check them against the code in this project, and suggest notes (and fixes to outdated ones) for the user to approve in SayQL.";

/// The connection and database an audit is for.
#[derive(Debug, Clone, PartialEq)]
pub struct Target<'a> {
    pub connection_id: &'a str,
    pub name: &'a str,
    /// `None`: a SQLite file, or a connection without a default database.
    pub database: Option<&'a str>,
}

/// The database notes are kept under: the one named, else the connection's default, as the
/// app saves them (its tabs name the default database), so both see the same notes.
pub fn notes_database(
    store: &Store,
    connection_id: &str,
    database: Option<&str>,
) -> Option<String> {
    match database.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => Some(d.to_string()),
        None => store
            .connection(connection_id)
            .ok()
            .map(|c| c.database.trim().to_string())
            .filter(|d| !d.is_empty()),
    }
}

/// The database an audit of `conn` is for: none for a SQLite file (one database, its notes
/// kept without one), else as [`notes_database`].
pub fn target_database(store: &Store, conn: &ConnConfig, database: Option<&str>) -> Option<String> {
    if conn.kind == crate::db::Kind::Sqlite {
        None
    } else {
        notes_database(store, &conn.id, database)
    }
}

/// The saved connection `wanted` names: its id, else its name (any case).
pub fn find_connection<'a>(list: &'a [ConnConfig], wanted: &str) -> Result<&'a ConnConfig, String> {
    let wanted = wanted.trim();
    if let Some(c) = list.iter().find(|c| c.id == wanted) {
        return Ok(c);
    }
    let named: Vec<&ConnConfig> = list
        .iter()
        .filter(|c| c.name.trim().eq_ignore_ascii_case(wanted))
        .collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<String> = list.iter().map(|c| format!("{:?}", c.name)).collect();
            Err(if names.is_empty() {
                "No connections are saved in SayQL yet.".to_string()
            } else {
                format!(
                    "No saved connection is called or has the id {wanted:?}. Saved connections: {}.",
                    names.join(", ")
                )
            })
        }
        _ => Err(format!(
            "More than one saved connection is called {wanted:?}; pass its id from list_connections."
        )),
    }
}

/// A knowledge source as the audit is told about it: `label (kind, origin @ version)`, and
/// where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Source<'a> {
    /// The short name evidence cites it by: `label:path@commit`, `label:page@version`.
    pub label: &'a str,
    /// What it is: "folder", "git repository", "files", "web page(s)".
    pub kind: &'a str,
    /// Documents (files, web pages) rather than code: they can be out of date.
    pub document: bool,
    /// The folder on this computer the agent reads.
    pub dir: &'a str,
    /// The repository's or web page's address.
    pub url: Option<&'a str>,
    /// The commit, or for documents the version of their text.
    pub commit: Option<&'a str>,
}

/// The audit instructions. Without a target the app picks the connection first. `copied`:
/// for pasting into an AI app by hand, so it says which tools to use.
pub fn instructions(target: Option<&Target>, copied: bool) -> String {
    instructions_for(target, copied, &[])
}

/// "backend (git repository, https://github.com/acme/api @ 1a2b3c4d): /path/to/copy". A
/// folder is read where it is; SayQL's copies are named with where they came from.
fn source_line(s: &Source) -> String {
    let at = match s.commit {
        Some(c) => format!(" @ {}", &c[..c.len().min(10)]),
        None => String::new(),
    };
    match s.url {
        Some(url) => format!("- {} ({}, {url}{at}): {}\n", s.label, s.kind, s.dir),
        None if s.document => format!("- {} ({}{at}): {}\n", s.label, s.kind, s.dir),
        None => format!("- {} ({}, {}{at})\n", s.label, s.kind, s.dir),
    }
}

/// How documents are cited, and that they lose to the code and the database.
fn documents_note(sources: &[Source]) -> String {
    let Some(doc) = sources.iter().find(|s| s.document) else {
        return String::new();
    };
    let example = doc
        .commit
        .map_or(String::new(), |c| format!("@{}", &c[..c.len().min(7)]));
    format!(
        "Some sources are documents (files, web pages): cite them as `label:page@version`, such as \
         `{}:billing/refunds.txt{example}`. Documents can be out of date: base notes on the code and the database. \
         Where a document disagrees with them, don't choose between the two: say what each says in the evidence, \
         and list the conflict in your summary.\n\n",
        doc.label
    )
}

/// [`instructions`] for a run over `sources`, the code SayQL gave the agent (none: the
/// project the AI app is working in).
pub fn instructions_for(target: Option<&Target>, copied: bool, sources: &[Source]) -> String {
    let mut out = String::new();
    let here = if sources.is_empty() {
        "the code in this project"
    } else {
        "its code"
    };
    let args = match target {
        Some(t) => {
            let db = match t.database {
                Some(d) => format!(", database {d:?}"),
                None => String::new(),
            };
            out.push_str(&format!(
                "Audit the database of the SayQL connection {:?} (connection_id {:?}{db}) against {here}, and suggest \
                 notes that teach SayQL what the schema doesn't say.\n\n",
                t.name, t.connection_id
            ));
            let db = match t.database {
                Some(d) => format!(" and database {d:?}"),
                None => String::new(),
            };
            format!("connection_id {:?}{db}", t.connection_id)
        }
        None => {
            out.push_str(
                "Audit one of the user's SayQL databases against the code in this project, and suggest notes that \
                 teach SayQL what the schema doesn't say.\n\n\
                 First pick the database: call list_connections, and find the connection this project's code uses (its \
                 database settings or .env.example; never read or print passwords or other secrets). For a server \
                 connection, call list_databases and pick the database the code uses. If more than one could fit, or \
                 none clearly does, ask the user which one to audit.\n\n",
            );
            "the connection_id (and database) you picked".to_string()
        }
    };
    if copied {
        out.push_str(
            "Use the SayQL MCP server's tools for this (describe_schema, suggest_note). If you don't have them, SayQL \
             isn't connected to this app yet: tell the user to set it up in SayQL under Settings → AI apps → Access.\n\n",
        );
    }
    let (code, evidence) = if sources.is_empty() {
        (
            "this project's code (the repository you're working in) ".to_string(),
            "file paths, short commit hashes, usage figures".to_string(),
        )
    } else {
        let list: String = sources.iter().map(source_line).collect();
        let what = if sources.iter().any(|s| s.document) {
            if sources.iter().all(|s| s.document) {
                "the documents"
            } else {
                "the code and documents"
            }
        } else {
            "the code"
        };
        let evidence = match sources.iter().find(|s| !s.document) {
            Some(code) => {
                let example = code
                    .commit
                    .map_or(String::new(), |c| format!("@{}", &c[..c.len().min(7)]));
                format!(
                    "files as `label:path@commit`, such as `{}:src/db/orders.ts{example}`, and usage figures",
                    code.label
                )
            }
            None => "documents as `label:page@version`, and usage figures".to_string(),
        };
        (
            format!("{what} in these sources (read them all):\n{list}"),
            evidence,
        )
    };
    out.push_str(&documents_note(sources));
    out.push_str(&format!(
        "1. Call describe_schema with {args}. For a large database it lists table names only: call it again with \
         `search` words for the tables you look into. Where the database keeps statistics it shows each table's usage \
         (reads, writes, last write, and the period they cover; SQLite keeps none), and it ends with the notes already \
         saved, each with an id such as [n_1a2b3c4d].\n\
         2. Search {code}for where each table and its main columns are used: queries, ORM models, migrations, raw SQL, \
         jobs and scripts.\n\
         3. Look for what the schema doesn't say:\n\
         - tables that are unused or retired: no references in the code and no writes in the usage period (say so \
         when usage isn't available), and what replaced them;\n\
         - what status and type codes, enum values and flags mean, as the code defines them;\n\
         - relationships the code relies on that aren't declared as foreign keys;\n\
         - units, currencies, time zones and conventions such as soft deletes (amounts in cents, timestamps in UTC, \
         deleted_at set means deleted).\n\
         4. Check the notes already saved against the code and the usage. For one that's wrong or outdated, call \
         suggest_note with `replaces` set to its id and the corrected note as `text`. For one that no longer applies, \
         call suggest_note with `replaces` set to its id, `remove: true`, and why as `text`.\n\
         5. For each finding, call suggest_note with {args}, the note as `text` (at most 300 characters) and your \
         evidence (at most 600 characters): {evidence}. When a query can confirm the note, add it as `check`: one \
         read-only SQL statement returning exactly one row with one column named `ok`, true while the note holds \
         (`SELECT count(*) = 0 AS ok FROM orders WHERE status NOT IN ('p', 'r')`); leave it out otherwise. Notes are schema-level \
         facts: never values from rows or personal data (names, emails, amounts, ids). Status codes and enum names \
         (`1=pending`, `status IN (2,3)`), SQL, file paths and units are fine. SayQL refuses suggestions that \
         look like values. You need no rows for this; if you run a query to check something, keep its results out of the notes.\n\
         6. Keep to the 10 to 20 most useful notes: facts that change how someone writes a query. Skip what the schema \
         already says (types, declared foreign keys) and notes already saved.\n\
         7. Finish with a short summary for the user: how many notes you suggested, how many replace or remove saved \
         ones, and anything you weren't sure of. Tell them the suggestions wait in SayQL (under Notes in the \
         navigator, and in Settings → AI apps → Suggestions) and nothing is saved until they accept it.\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Kind;

    fn conn(id: &str, name: &str) -> ConnConfig {
        ConnConfig {
            id: id.into(),
            name: name.into(),
            kind: Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: "/tmp/x.sqlite".into(),
            ssl: false,
            trust_cert: false,
            production: false,
        }
    }

    #[test]
    fn finds_a_connection_by_id_or_name() {
        let list = [
            conn("c1", "Shop"),
            conn("c2", "Analytics"),
            conn("c3", "shop "),
        ];
        assert_eq!(find_connection(&list, "c2").unwrap().id, "c2");
        assert_eq!(find_connection(&list, " analytics").unwrap().id, "c2");
        assert!(
            find_connection(&list, "shop")
                .unwrap_err()
                .contains("More than one")
        );
        let e = find_connection(&list, "crm").unwrap_err();
        assert!(e.contains("\"Shop\"") && e.contains("\"Analytics\""), "{e}");
        assert!(
            find_connection(&[], "x")
                .unwrap_err()
                .contains("No connections")
        );
    }

    #[test]
    fn instructions_name_the_target_and_the_tools() {
        let t = Target {
            connection_id: "c1",
            name: "Shop",
            database: Some("sales"),
        };
        let text = instructions(Some(&t), false);
        assert!(text.starts_with("Audit the database of the SayQL connection \"Shop\" (connection_id \"c1\", database \"sales\")"));
        for word in [
            "describe_schema",
            "suggest_note",
            "`replaces`",
            "`remove: true`",
            "never values from rows",
            "10 to 20",
        ] {
            assert!(text.contains(word), "{word}");
        }
        assert!(text.contains("connection_id \"c1\" and database \"sales\""));
        assert!(
            !text.contains("SayQL MCP server's tools"),
            "MCP apps have the tools"
        );
        assert!(instructions(Some(&t), true).contains("SayQL MCP server's tools"));
        let open = instructions(None, false);
        assert!(open.contains("call list_connections") && open.contains("ask the user"));
        // Runs read-only, without commands: nothing asks for git log.
        assert!(!text.contains("git log"));
        assert!(text.contains("this project's code (the repository you're working in) for where"));
        assert!(text.contains("`check`") && text.contains("named `ok`"));
    }

    #[test]
    fn instructions_list_the_code_sources() {
        let t = Target {
            connection_id: "c1",
            name: "Shop",
            database: None,
        };
        let text = instructions_for(
            Some(&t),
            false,
            &[
                Source {
                    label: "backend",
                    kind: "git repository",
                    document: false,
                    dir: "/app/repos/src_1",
                    url: Some("https://github.com/acme/api.git"),
                    commit: Some("1a2b3c4d5e6f7a8b9c0d"),
                },
                Source {
                    label: "web",
                    kind: "folder",
                    document: false,
                    dir: "/Users/me/web",
                    url: None,
                    commit: None,
                },
            ],
        );
        assert!(text.contains(
            "the code in these sources (read them all):\n- backend (git repository, https://github.com/acme/api.git @ 1a2b3c4d5e): /app/repos/src_1\n- web (folder, /Users/me/web)\nfor where"
        ), "{text}");
        assert!(text.contains("`label:path@commit`, such as `backend:src/db/orders.ts@1a2b3c4`"));
        assert!(!text.contains("this project's code"));
        assert!(!text.contains("git log"));
        assert!(!text.contains("label:page@version"), "no documents");
    }

    #[test]
    fn documents_are_cited_by_page_and_lose_to_the_code() {
        let t = Target {
            connection_id: "c1",
            name: "Shop",
            database: None,
        };
        let code = Source {
            label: "backend",
            kind: "git repository",
            document: false,
            dir: "/app/repos/src_1",
            url: Some("https://github.com/acme/api.git"),
            commit: Some("1a2b3c4d5e6f"),
        };
        let docs = Source {
            label: "docs",
            kind: "web pages",
            document: true,
            dir: "/app/repos/src_2",
            url: Some("https://docs.acme.com/db/"),
            commit: Some("ab12cd34ef56ab78"),
        };
        let specs = Source {
            label: "specs",
            kind: "files",
            document: true,
            dir: "/app/repos/src_3",
            url: None,
            commit: Some("99ff"),
        };
        let text = instructions_for(Some(&t), false, &[code.clone(), docs.clone(), specs]);
        assert!(
            text.contains("the code and documents in these sources (read them all):\n"),
            "{text}"
        );
        assert!(text.contains(
            "- docs (web pages, https://docs.acme.com/db/ @ ab12cd34ef): /app/repos/src_2\n"
        ));
        assert!(text.contains("- specs (files @ 99ff): /app/repos/src_3\n"));
        assert!(text.contains("`label:page@version`, such as `docs:billing/refunds.txt@ab12cd3`"));
        assert!(
            text.contains("Documents can be out of date: base notes on the code and the database")
        );
        assert!(
            text.contains("don't choose between the two") && text.contains("list the conflict")
        );
        assert!(text.contains("`backend:src/db/orders.ts@1a2b3c4`"));
        let only_docs = instructions_for(Some(&t), false, &[docs]);
        assert!(only_docs.contains("the documents in these sources"));
        assert!(only_docs.contains("documents as `label:page@version`, and usage figures"));
    }
}
