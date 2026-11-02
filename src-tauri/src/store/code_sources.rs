//! The knowledge an audit reads, per connection: local folders and git repositories (any host),
//! files the person picked, and web pages. Only where each is and how its last fetch went is
//! kept here; a repository's token, if it needs one, is in the keychain
//! ([`crate::store::secrets::repo_token`]), and its copy (or the text of its files and pages)
//! is in `repos/<source id>` next to this file (see [`crate::repos`], [`crate::knowledge`]).
//! The file keeps its first name, `code-sources.json`.
//!
//! Replaces `audit-folders.json` (one folder per connection), whose folders become local
//! sources the first time this file is read.

use super::Store;
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const FILE: &str = "code-sources.json";
/// The one-folder-per-connection file this replaces.
const OLD_FILE: &str = "audit-folders.json";
/// Where git sources are copied, next to the settings files.
const REPOS_DIR: &str = "repos";
/// Sources one connection can have in the free app. SayQL for teams has no limit.
pub const MAX_SOURCES_FREE: usize = 3;
/// Why a 4th source is refused. The window links to SayQL for teams after it.
pub const CAP_MESSAGE: &str = "SayQL for teams keeps unlimited sources in sync for everyone.";

/// Where a source's knowledge is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceKind {
    /// A folder on this computer, read where it is.
    Local { path: String },
    /// A git repository, copied by SayQL with the person's own git setup.
    Git {
        url: String,
        /// Empty: the repository's default branch.
        #[serde(default)]
        branch: String,
        /// Only this folder of the repository is read.
        #[serde(default)]
        subdir: Option<String>,
        /// A read-only token is saved in the keychain for it.
        #[serde(default)]
        has_token: bool,
        /// The user name sent with the token, when its host isn't one SayQL knows.
        #[serde(default)]
        token_user: Option<String>,
    },
    /// Files on this computer (documents, exports), read again on each refresh.
    Files {
        #[serde(default)]
        paths: Vec<String>,
    },
    /// A web page, or the pages in its folder on the same site when `crawl`.
    Url {
        #[serde(default)]
        url: String,
        #[serde(default)]
        crawl: bool,
    },
}

impl SourceKind {
    /// Code (a folder or repository) rather than documents.
    pub fn is_code(&self) -> bool {
        matches!(self, SourceKind::Local { .. } | SourceKind::Git { .. })
    }

    /// What the audit is told the source is.
    pub fn name(&self) -> &'static str {
        match self {
            SourceKind::Local { .. } => "folder",
            SourceKind::Git { .. } => "git repository",
            SourceKind::Files { .. } => "files",
            SourceKind::Url { crawl: false, .. } => "web page",
            SourceKind::Url { crawl: true, .. } => "web pages",
        }
    }
}

/// One knowledge source of a connection (named for its first kind, code).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSource {
    /// `src_` and 12 hex digits: also the name of its copy's folder.
    pub id: String,
    /// A short name the audit cites files by (`backend:src/db.ts`), unique per connection.
    pub label: String,
    pub kind: SourceKind,
    /// When it was last fetched (git) or found (a folder), in ms since the Unix epoch.
    #[serde(default)]
    pub last_fetch: Option<u64>,
    /// The commit read last time, when the source is a git repository; for files and web
    /// pages, the version of their text (a sha256 over it).
    #[serde(default)]
    pub last_commit: Option<String>,
    /// Why the last fetch or check failed; `None` when it worked.
    #[serde(default)]
    pub error: Option<String>,
}

impl KnowledgeSource {
    /// A new source with a fresh id; `label` is made unique among `others`.
    pub fn new(kind: SourceKind, label: &str, others: &[KnowledgeSource]) -> Self {
        let id = format!("src_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        KnowledgeSource {
            id,
            label: unique_label(label, others),
            kind,
            last_fetch: None,
            last_commit: None,
            error: None,
        }
    }

    pub fn is_git(&self) -> bool {
        matches!(self.kind, SourceKind::Git { .. })
    }

    /// SayQL keeps its copy in `repos/<id>`: a repository, or the text of files or pages.
    pub fn has_copy(&self) -> bool {
        !matches!(self.kind, SourceKind::Local { .. })
    }
}

/// A label from a folder path or repository URL: its last part, without `.git`.
pub fn default_label(path_or_url: &str) -> String {
    let last = path_or_url
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("");
    let last = last.strip_suffix(".git").unwrap_or(last);
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches(['-', '.']).to_string();
    if clean.is_empty() {
        "code".into()
    } else {
        clean
    }
}

/// `label` cleaned up, with `-2`, `-3`… added when another source has it.
fn unique_label(label: &str, others: &[KnowledgeSource]) -> String {
    let base = default_label(label);
    let taken = |l: &str| others.iter().any(|o| o.label.eq_ignore_ascii_case(l));
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|l| !taken(l))
        .expect("some number is free")
}

/// Ids are made here, but the file can be edited: only these name a folder.
fn valid_id(id: &str) -> bool {
    id.len() > 4
        && id.len() <= 40
        && id.starts_with("src_")
        && id[4..].chars().all(|c| c.is_ascii_alphanumeric())
}

impl Store {
    fn all_sources(&self) -> AppResult<BTreeMap<String, Vec<KnowledgeSource>>> {
        if !self.dir.join(FILE).exists() {
            // Saves the first sources, so their ids stay the same from now on.
            return self.update_sources(|all| Ok(all.clone()));
        }
        self.read(FILE)
    }

    /// Changes the sources under the file's lock (see [`Store::update`]), starting from the
    /// old audit folders the first time.
    fn update_sources<R>(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, Vec<KnowledgeSource>>) -> AppResult<R>,
    ) -> AppResult<R> {
        self.update_or(FILE, || self.migrate_audit_folders(), change)
    }

    /// The first read: each connection's audit folder becomes its first source. The old file
    /// is left in place for an older SayQL.
    fn migrate_audit_folders(&self) -> AppResult<BTreeMap<String, Vec<KnowledgeSource>>> {
        let old: BTreeMap<String, String> = self.read(OLD_FILE)?;
        let all: BTreeMap<String, Vec<KnowledgeSource>> = old
            .into_iter()
            .filter(|(_, path)| !path.trim().is_empty())
            .map(|(conn, path)| {
                let source = KnowledgeSource::new(
                    SourceKind::Local { path: path.clone() },
                    &default_label(&path),
                    &[],
                );
                (conn, vec![source])
            })
            .collect();
        Ok(all)
    }

    /// This connection's code sources, in the order added (the first is where the audit runs).
    pub fn knowledge_sources(&self, connection_id: &str) -> AppResult<Vec<KnowledgeSource>> {
        Ok(self
            .all_sources()?
            .remove(connection_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| valid_id(&s.id))
            .collect())
    }

    pub fn knowledge_source(
        &self,
        connection_id: &str,
        source_id: &str,
    ) -> AppResult<KnowledgeSource> {
        self.knowledge_sources(connection_id)?
            .into_iter()
            .find(|s| s.id == source_id)
            .ok_or_else(|| AppError::User("That source was removed.".into()))
    }

    /// Adds a source for this connection; the same source twice is refused, and so is one
    /// past [`MAX_SOURCES_FREE`].
    pub fn add_knowledge_source(
        &self,
        connection_id: &str,
        kind: SourceKind,
        label: &str,
    ) -> AppResult<KnowledgeSource> {
        self.update_sources(|all| {
            let list = all.entry(connection_id.to_string()).or_default();
            let same = |a: &SourceKind, b: &SourceKind| match (a, b) {
                (SourceKind::Local { path: x }, SourceKind::Local { path: y }) => x == y,
                (
                    SourceKind::Git {
                        url: u1,
                        branch: b1,
                        subdir: s1,
                        ..
                    },
                    SourceKind::Git {
                        url: u2,
                        branch: b2,
                        subdir: s2,
                        ..
                    },
                ) => u1 == u2 && b1 == b2 && s1 == s2,
                (SourceKind::Files { paths: x }, SourceKind::Files { paths: y }) => {
                    let (mut x, mut y) = (x.clone(), y.clone());
                    x.sort();
                    y.sort();
                    x == y
                }
                (SourceKind::Url { url: x, .. }, SourceKind::Url { url: y, .. }) => x == y,
                _ => false,
            };
            if list.iter().any(|s| same(&s.kind, &kind)) {
                return Err("This connection already has that source.".into());
            }
            if list.len() >= MAX_SOURCES_FREE {
                return Err(CAP_MESSAGE.into());
            }
            let source = KnowledgeSource::new(kind, label, list);
            list.push(source.clone());
            Ok(source)
        })
    }

    /// Changes one source in place (its status after a fetch); a removed one is left alone.
    pub fn update_knowledge_source(
        &self,
        connection_id: &str,
        source_id: &str,
        change: impl FnOnce(&mut KnowledgeSource),
    ) -> AppResult<Option<KnowledgeSource>> {
        self.update_sources(|all| {
            let Some(source) = all
                .get_mut(connection_id)
                .and_then(|l| l.iter_mut().find(|s| s.id == source_id))
            else {
                return Ok(None);
            };
            change(source);
            Ok(Some(source.clone()))
        })
    }

    /// Removes a source with its token and its copy.
    pub fn remove_knowledge_source(&self, connection_id: &str, source_id: &str) -> AppResult<()> {
        let gone = self.update_sources(|all| {
            let Some(list) = all.get_mut(connection_id) else {
                return Ok(None);
            };
            let Some(i) = list.iter().position(|s| s.id == source_id) else {
                return Ok(None);
            };
            let gone = list.remove(i);
            if list.is_empty() {
                all.remove(connection_id);
            }
            Ok(Some(gone))
        })?;
        // The token and the copy go after the file is saved and its lock is released.
        match gone {
            Some(gone) => self.drop_source_files(&gone),
            None => Ok(()),
        }
    }

    /// Where a source's copy is kept: `repos/<source id>`.
    pub fn repo_dir(&self, source_id: &str) -> AppResult<PathBuf> {
        if !valid_id(source_id) {
            return Err("That source isn't valid.".into());
        }
        Ok(self.dir.join(REPOS_DIR).join(source_id))
    }

    /// A removed source's token and copy. The keychain is only touched when a token was saved.
    fn drop_source_files(&self, source: &KnowledgeSource) -> AppResult<()> {
        if let SourceKind::Git {
            has_token: true, ..
        } = source.kind
        {
            super::secrets::delete_repo_token(&source.id)?;
        }
        if source.has_copy()
            && let Ok(dir) = self.repo_dir(&source.id)
            && dir.exists()
        {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Forgets a deleted connection's sources, their tokens and copies.
    pub(super) fn forget_knowledge_sources(&self, connection_id: &str) -> AppResult<()> {
        let gone = self
            .update_sources(|all| Ok(all.remove(connection_id)))?
            .unwrap_or_default();
        gone.iter()
            .map(|s| self.drop_source_files(s))
            .fold(Ok(()), AppResult::and)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> (PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("sayql-{tag}-{}", uuid::Uuid::new_v4()));
        (dir.clone(), Store::new(dir))
    }

    fn git(url: &str) -> SourceKind {
        SourceKind::Git {
            url: url.into(),
            branch: "main".into(),
            subdir: None,
            has_token: false,
            token_user: None,
        }
    }

    impl SourceKind {
        fn with_branch(mut self, b: &str) -> Self {
            if let SourceKind::Git { branch, .. } = &mut self {
                *branch = b.into();
            }
            self
        }
    }

    #[test]
    fn audit_folders_become_sources_once() {
        let (dir, s) = store("sources-migrate");
        std::fs::write(
            dir.join(OLD_FILE),
            r#"{"c1":"/Users/me/code/shop","c2":"/Users/me/code/crm/"}"#,
        )
        .unwrap();
        let c1 = s.knowledge_sources("c1").unwrap();
        assert_eq!(c1.len(), 1);
        assert_eq!(
            c1[0].kind,
            SourceKind::Local {
                path: "/Users/me/code/shop".into()
            }
        );
        assert_eq!(c1[0].label, "shop");
        assert_eq!(s.knowledge_sources("c2").unwrap()[0].label, "crm");
        // Once: a later change to the old file isn't read again, and removing isn't undone.
        s.remove_knowledge_source("c1", &c1[0].id).unwrap();
        std::fs::write(dir.join(OLD_FILE), r#"{"c3":"/x"}"#).unwrap();
        assert!(s.knowledge_sources("c1").unwrap().is_empty());
        assert!(s.knowledge_sources("c3").unwrap().is_empty());
        assert_eq!(
            Store::new(dir.clone()).knowledge_sources("c2").unwrap()[0].id,
            s.knowledge_sources("c2").unwrap()[0].id
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sources_per_connection_with_unique_labels() {
        let (dir, s) = store("sources");
        let a = s
            .add_knowledge_source("c1", git("https://github.com/acme/api.git"), "api")
            .unwrap();
        let b = s
            .add_knowledge_source("c1", git("git@gitlab.com:acme/api.git"), "api")
            .unwrap();
        assert_eq!((a.label.as_str(), b.label.as_str()), ("api", "api-2"));
        assert!(valid_id(&a.id) && a.id != b.id);
        assert!(
            s.add_knowledge_source("c1", git("https://github.com/acme/api.git"), "x")
                .is_err(),
            "the same repository twice"
        );
        s.add_knowledge_source(
            "c2",
            SourceKind::Local {
                path: "/code".into(),
            },
            "code",
        )
        .unwrap();
        assert_eq!(s.knowledge_sources("c1").unwrap().len(), 2);

        let updated = s
            .update_knowledge_source("c1", &a.id, |x| {
                x.last_commit = Some("abc123".into());
                x.error = None;
            })
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_commit.as_deref(), Some("abc123"));
        assert_eq!(
            s.update_knowledge_source("c1", "src_gone", |_| {}).unwrap(),
            None
        );

        // A source's copy goes with it, and so do a deleted connection's.
        let copy = s.repo_dir(&a.id).unwrap();
        std::fs::create_dir_all(copy.join("src")).unwrap();
        s.remove_knowledge_source("c1", &a.id).unwrap();
        assert!(!copy.exists());
        let copy_b = s.repo_dir(&b.id).unwrap();
        std::fs::create_dir_all(&copy_b).unwrap();
        s.forget_agent("c1").unwrap();
        assert!(s.knowledge_sources("c1").unwrap().is_empty());
        assert!(!copy_b.exists());
        assert_eq!(s.knowledge_sources("c2").unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_removed_source_takes_its_token_with_it() {
        let (dir, s) = store("sources-token");
        let mut kind = git("https://github.com/acme/private.git");
        if let SourceKind::Git { has_token, .. } = &mut kind {
            *has_token = true;
        }
        let src = s.add_knowledge_source("c1", kind, "private").unwrap();
        super::super::secrets::set_repo_token(&src.id, "ghp_test").unwrap();
        assert!(super::super::secrets::repo_token(&src.id).is_some());
        s.forget_agent("c1").unwrap();
        assert_eq!(super::super::secrets::repo_token(&src.id), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn three_sources_per_connection_in_the_free_app() {
        let (dir, s) = store("sources-cap");
        let docs = SourceKind::Url {
            url: "https://docs.acme.com/db/".into(),
            crawl: true,
        };
        s.add_knowledge_source("c1", git("https://github.com/acme/api.git"), "api")
            .unwrap();
        s.add_knowledge_source("c1", docs.clone(), "docs").unwrap();
        let files = SourceKind::Files {
            paths: vec!["/b.md".into(), "/a.pdf".into()],
        };
        s.add_knowledge_source("c1", files, "specs").unwrap();
        let same_files = SourceKind::Files {
            paths: vec!["/a.pdf".into(), "/b.md".into()],
        };
        assert!(
            s.add_knowledge_source("c1", same_files, "x")
                .unwrap_err()
                .to_string()
                .contains("already has that source")
        );
        let e = s
            .add_knowledge_source("c1", git("https://github.com/acme/web.git"), "web")
            .unwrap_err();
        assert_eq!(e.to_string(), CAP_MESSAGE);
        assert_eq!(s.knowledge_sources("c1").unwrap().len(), MAX_SOURCES_FREE);
        // Per connection, and a removed one makes room.
        s.add_knowledge_source("c2", docs, "docs").unwrap();
        let first = s.knowledge_sources("c1").unwrap()[0].id.clone();
        s.remove_knowledge_source("c1", &first).unwrap();
        s.add_knowledge_source("c1", git("https://github.com/acme/web.git"), "web")
            .unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sources_saved_by_an_older_sayql_still_load() {
        let (dir, s) = store("sources-old");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"{"c1":[
                {"id":"src_aaaaaaaaaaaa","label":"shop","kind":{"type":"local","path":"/code/shop"},"lastFetch":1,"lastCommit":null,"error":null},
                {"id":"src_bbbbbbbbbbbb","label":"api","kind":{"type":"git","url":"https://github.com/acme/api.git"}},
                {"id":"src_cccccccccccc","label":"docs","kind":{"type":"url"}}
            ]}"#,
        )
        .unwrap();
        let list = s.knowledge_sources("c1").unwrap();
        assert_eq!(list[0].kind.name(), "folder");
        assert_eq!(
            list[1].kind,
            git("https://github.com/acme/api.git").with_branch("")
        );
        assert_eq!(
            list[2].kind,
            SourceKind::Url {
                url: String::new(),
                crawl: false
            }
        );
        // A files source's copy goes with it; a folder is never touched.
        let files = s
            .add_knowledge_source(
                "c2",
                SourceKind::Files {
                    paths: vec!["/x.md".into()],
                },
                "x",
            )
            .unwrap();
        let copy = s.repo_dir(&files.id).unwrap();
        std::fs::create_dir_all(&copy).unwrap();
        s.remove_knowledge_source("c2", &files.id).unwrap();
        assert!(!copy.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn labels_and_ids() {
        assert_eq!(
            default_label("https://github.com/acme/billing-api.git"),
            "billing-api"
        );
        assert_eq!(default_label("git@github.com:acme/shop.git"), "shop");
        assert_eq!(default_label("/Users/me/My Code/"), "My-Code");
        assert_eq!(default_label(r"C:\code\crm"), "crm");
        assert_eq!(default_label("///"), "code");
        for bad in ["src_", "src_../x", "../src_a", "src_a/b", "x_abc"] {
            assert!(!valid_id(bad), "{bad}");
        }
        let (dir, s) = store("sources-ids");
        assert!(s.repo_dir("src_../../etc").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
