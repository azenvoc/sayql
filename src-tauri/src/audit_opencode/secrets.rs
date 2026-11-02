//! Files the built-in OpenCode never reads or searches: one list ([`SECRETS`]) that becomes
//! OpenCode's `read` deny rules ([`secret_files`]), the ripgrep excludes ([`rg_secret_globs`])
//! and a scan of the sources ([`secret_paths`]), so the three can't drift apart.
//!
//! Case: OpenCode 1.19.5 matches permission patterns case-sensitively except on Windows
//! (`packages/core/src/util/wildcard.ts:13`, flags `"si"` on win32 and `"s"` elsewhere), and has
//! no character classes (`[` and `]` are escaped, line 7), so a pattern can't say "any case".
//! The pattern is the path as the model asked for it, not as it is on disk
//! (`packages/opencode/src/tool/read.ts:234-257`: `path.relative(worktree, filepath)`, resolved
//! with `realpath` only on Windows). On a case-insensitive disk (macOS by default) the model can
//! read `.env` by asking for `.ENV`, and on any disk a file named `.ENV` or `ID_RSA` misses the
//! rules. So before each run SayQL scans the sources for names that look like secrets in any
//! case and denies each one found by its shape: every ASCII letter of its path in the source
//! becomes `?` (one character, any case), so every spelling of that path is denied. A shape can
//! also match a same-length name beside it; that only hides more. Refusing the run instead would
//! refuse every source with an ordinary `.env` on a Mac. ripgrep has `--iglob`, so searches
//! exclude them case-insensitively directly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where a secret is, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secret {
    /// A file name (`*` any run of characters, `?` one), in any folder.
    Name(&'static str),
    /// A path ending, `/` between folders.
    Ending(&'static str),
    /// A folder, nothing in which is read.
    Folder(&'static str),
}

use Secret::{Ending, Folder, Name};

/// Every secret file. Names that are mostly source code elsewhere (`secrets.ts`, `keys.py`)
/// aren't here: an audit has to read code.
pub const SECRETS: [Secret; 34] = [
    Name("*.env"),
    Name("*.env.*"),
    Name(".envrc"),
    Name(".dev.vars"),
    Name("*.pem"),
    Name("*.key"),
    Name("*.p12"),
    Name("*.pfx"),
    Name("*.ppk"),
    Name("*.jks"),
    Name("*.keystore"),
    Name("*.kdbx"),
    Name("*id_rsa*"),
    Name("*id_dsa*"),
    Name("*id_ecdsa*"),
    Name("*id_ed25519*"),
    Name("*.npmrc"),
    Name("*.netrc"),
    Name(".pypirc"),
    Name(".pgpass"),
    Name(".my.cnf"),
    Name("*credentials*"),
    Name("*service-account*.json"),
    Name("*.tfstate"),
    Name("*.tfstate.*"),
    Name("*.tfvars"),
    Name("*.tfvars.json"),
    Name("kubeconfig"),
    Name("*.kubeconfig"),
    Name("secrets.json"),
    Name("secrets.yaml"),
    Name("secrets.yml"),
    Ending(".docker/config.json"),
    Folder(".git"),
];

/// Read anyway, as in OpenCode's own defaults: an example file names settings without values.
pub const READABLE: &str = "*.env.example";

/// [`SECRETS`] as OpenCode `read` patterns. A pattern's `*` matches across folders, so a name
/// starting with `*` covers every folder by itself; any other name is given at the top and
/// under any folder.
pub fn secret_files() -> Vec<String> {
    let mut out = Vec::new();
    for s in SECRETS {
        match s {
            Name(n) if n.starts_with('*') => out.push(n.to_string()),
            Name(n) | Ending(n) => {
                out.push(n.to_string());
                out.push(format!("*/{n}"));
            }
            Folder(f) => {
                out.push(format!("{f}/*"));
                out.push(format!("*/{f}/*"));
            }
        }
    }
    out
}

/// [`SECRETS`] as ripgrep globs, for `--iglob=!<glob>` (case-insensitive). A glob without a
/// slash matches the file name in any folder.
pub fn rg_secret_globs() -> Vec<String> {
    SECRETS
        .iter()
        .map(|s| match s {
            Name(n) => n.to_string(),
            Ending(p) => format!("**/{p}"),
            Folder(f) => format!("**/{f}/**"),
        })
        .collect()
}

/// `name` against `pattern` (`*` any run, `?` one character), ignoring case as a
/// case-insensitive disk would (Unicode lowercase, so the Kelvin sign matches `k`).
fn wild(name: &[char], pattern: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|k| name.get(k..).is_some_and(|n| wild(n, rest))),
        Some(('?', rest)) => name.split_first().is_some_and(|(_, n)| wild(n, rest)),
        Some((p, rest)) => name
            .split_first()
            .is_some_and(|(c, n)| same(*c, *p) && wild(n, rest)),
    }
}

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

fn matches(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    wild(&t, &p)
}

/// Whether a file at `rel` (`/` between folders) looks like a secret, in any case.
pub fn is_secret_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if matches(name, READABLE) {
        return false;
    }
    SECRETS.iter().any(|s| match s {
        Name(n) => matches(name, n),
        Ending(p) => matches(rel, p) || matches(rel, &format!("*/{p}")),
        Folder(_) => false,
    })
}

/// Whether a folder named `name` is one nothing in which is read, in any case.
pub fn is_secret_folder(name: &str) -> bool {
    SECRETS
        .iter()
        .any(|s| matches!(s, Folder(f) if matches(name, f)))
}

/// A file or folder in a source that looks like a secret (or a link to one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hidden {
    /// Which source, by its place in the run's list.
    pub source: usize,
    /// Its path in that source, `/` between folders; a folder's ends with `/`.
    pub rel: String,
}

/// Everything in `sources` that looks like a secret by name, whatever its case: files, secret
/// folders (not looked into), links whose own name or whose target's name looks like one, and
/// what a link to a folder in the sources leads to (one link deep). Links out of the sources
/// are refused separately ([`super::link_out`]).
pub fn secret_paths(sources: &[PathBuf]) -> Vec<Hidden> {
    // (source, rel, where it really is)
    let mut found: Vec<(usize, String, Option<PathBuf>)> = Vec::new();
    // (source, rel of the link, the folder it leads to)
    let mut dir_links: Vec<(usize, String, PathBuf)> = Vec::new();
    for (source, root) in sources.iter().enumerate() {
        let mut stack: Vec<(PathBuf, String)> = vec![(root.clone(), String::new())];
        while let Some((dir, prefix)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let Ok(kind) = e.file_type() else { continue };
                let name = e.file_name().to_string_lossy().into_owned();
                let rel = format!("{prefix}{name}");
                let path = e.path();
                if kind.is_symlink() {
                    let target = path.canonicalize().ok();
                    let target_name = target
                        .as_deref()
                        .and_then(Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned());
                    let is_dir = target.as_deref().is_some_and(Path::is_dir);
                    let secret = if is_dir {
                        is_secret_folder(&name)
                            || target_name.as_deref().is_some_and(is_secret_folder)
                    } else {
                        is_secret_file(&rel) || target_name.as_deref().is_some_and(is_secret_file)
                    };
                    if secret {
                        let rel = if is_dir { format!("{rel}/") } else { rel };
                        found.push((source, rel, None));
                    } else if let (true, Some(t)) = (is_dir, target) {
                        dir_links.push((source, rel, t));
                    }
                } else if kind.is_dir() {
                    if is_secret_folder(&name) {
                        found.push((source, format!("{rel}/"), path.canonicalize().ok()));
                    } else {
                        stack.push((path, format!("{rel}/")));
                    }
                } else if is_secret_file(&rel) {
                    found.push((source, rel, path.canonicalize().ok()));
                }
            }
        }
    }
    let mut out: Vec<Hidden> = found
        .iter()
        .map(|(source, rel, _)| Hidden {
            source: *source,
            rel: rel.clone(),
        })
        .collect();
    for (source, link, target) in &dir_links {
        for (_, rel, real) in &found {
            let Some(inside) = real.as_deref().and_then(|r| r.strip_prefix(target).ok()) else {
                continue;
            };
            let parts: Vec<String> = inside
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if parts.is_empty() {
                continue;
            }
            let slash = if rel.ends_with('/') { "/" } else { "" };
            out.push(Hidden {
                source: *source,
                rel: format!("{link}/{}{slash}", parts.join("/")),
            });
        }
    }
    out
}

/// `rel`'s shape as an OpenCode pattern: each ASCII letter `?` (any case), digits and `._-/ `
/// as they are, any other ASCII character `?`, and anything else `*` (a character outside ASCII
/// may be two UTF-16 units, or stand for two letters on disk). A folder's trailing `/` becomes
/// `/*`.
pub fn shape(rel: &str) -> String {
    let mut out: String = rel
        .chars()
        .map(|c| match c {
            '0'..='9' | '.' | '_' | '-' | '/' | ' ' => c,
            c if c.is_ascii() => '?',
            _ => '*',
        })
        .collect();
    if out.ends_with('/') {
        out.push('*');
    }
    out
}

/// The `read` deny patterns for what [`secret_paths`] found, in order without repeats. OpenCode
/// matches the path relative to its worktree, which is `/` for the plain folders SayQL gives it
/// (`packages/opencode/src/project/project.ts:217`), so each is the source's path (both names of
/// a macOS `/private` folder) and the shape. In case the worktree is the working folder, the
/// shape alone covers the first source, and `*/<folder>/` and the shape the others.
pub fn hidden_rules(sources: &[PathBuf], hidden: &[Hidden]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for h in hidden {
        let Some(src) = sources.get(h.source) else {
            continue;
        };
        let tail = shape(&h.rel);
        let mut pats = Vec::new();
        if h.source == 0 {
            pats.push(tail.clone());
        } else if let Some(name) = src.file_name() {
            // From the working folder another source is `../<its folder>/...`.
            pats.push(format!("*/{}/{tail}", name.to_string_lossy()));
        }
        for d in super::source_names(src) {
            pats.push(format!("*{}/{tail}", d.trim_start_matches(['/', '\\'])));
        }
        for p in pats {
            if seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    out
}
