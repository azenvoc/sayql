//! Which coding agents look installed, so "Connect your coding agent" can preselect one.
//! Read-only: it checks whether files and folders exist and lists two extension folders. It never
//! runs or opens anything. The ids match `src/features/agent/agents.ts`.

use super::runner;
use std::path::{Path, PathBuf};

/// How to tell that one agent is installed. Any match counts.
struct Rule {
    id: &'static str,
    /// Executables on PATH or in the usual install folders (see `runner::search_dirs`).
    bins: &'static [&'static str],
    /// macOS app bundles, in /Applications or ~/Applications.
    mac_apps: &'static [&'static str],
    /// Folders under %LOCALAPPDATA% on Windows.
    win_dirs: &'static [&'static str],
    /// Folders under the home folder.
    home_dirs: &'static [&'static str],
    /// VS Code extension folder prefixes (in VS Code's or Cursor's extensions folder).
    extensions: &'static [&'static str],
}

const NONE: &[&str] = &[];

const RULES: &[Rule] = &[
    Rule {
        id: "claude-code",
        bins: &["claude"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "claude-desktop",
        bins: NONE,
        mac_apps: &["Claude.app"],
        win_dirs: &["AnthropicClaude"],
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "cursor",
        bins: &["cursor"],
        mac_apps: &["Cursor.app"],
        win_dirs: &["Programs/cursor"],
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "vscode",
        bins: &["code"],
        mac_apps: &["Visual Studio Code.app"],
        win_dirs: &["Programs/Microsoft VS Code"],
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "copilot-cli",
        bins: &["copilot"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "windsurf",
        bins: &["windsurf"],
        mac_apps: &["Windsurf.app"],
        win_dirs: &["Programs/Windsurf"],
        home_dirs: &[".codeium/windsurf"],
        extensions: NONE,
    },
    Rule {
        id: "opencode",
        bins: &["opencode"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: &[".opencode/bin/opencode"],
        extensions: NONE,
    },
    Rule {
        id: "codex",
        bins: &["codex"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "gemini",
        bins: &["gemini"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "zed",
        bins: &["zed"],
        mac_apps: &["Zed.app"],
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
    Rule {
        id: "cline",
        bins: NONE,
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: &["saoudrizwan.claude-dev-"],
    },
    Rule {
        id: "roo",
        bins: NONE,
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: &["rooveterinaryinc.roo-cline-"],
    },
    Rule {
        id: "continue",
        bins: NONE,
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: &["continue.continue-"],
    },
    Rule {
        id: "amp",
        bins: &["amp"],
        mac_apps: NONE,
        win_dirs: NONE,
        home_dirs: NONE,
        extensions: NONE,
    },
];

/// Where to look, and how; tests pass a made-up file system.
pub struct Probe<'a> {
    /// Folders searched for executables.
    pub bin_dirs: Vec<PathBuf>,
    /// Folders holding macOS app bundles (empty elsewhere).
    pub app_dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    /// %LOCALAPPDATA% (Windows only).
    pub local_appdata: Option<PathBuf>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    /// The names in a folder; empty when it can't be read.
    pub list: &'a dyn Fn(&Path) -> Vec<String>,
}

/// The ids of the agents that look installed, in the catalogue's order.
pub fn detect(p: &Probe) -> Vec<&'static str> {
    let extensions: Vec<String> = p
        .home
        .iter()
        .flat_map(|h| [h.join(".vscode/extensions"), h.join(".cursor/extensions")])
        .flat_map(|d| (p.list)(&d))
        .collect();
    RULES
        .iter()
        .filter(|r| {
            runner::find_in(r.bins, &p.bin_dirs, |f| (p.exists)(f)).is_some()
                || r.mac_apps
                    .iter()
                    .any(|a| p.app_dirs.iter().any(|d| (p.exists)(&d.join(a))))
                || p.local_appdata
                    .as_ref()
                    .is_some_and(|l| r.win_dirs.iter().any(|d| (p.exists)(&l.join(d))))
                || p.home
                    .as_ref()
                    .is_some_and(|h| r.home_dirs.iter().any(|d| (p.exists)(&h.join(d))))
                || r.extensions
                    .iter()
                    .any(|x| extensions.iter().any(|e| e.starts_with(x)))
        })
        .map(|r| r.id)
        .collect()
}

/// The agents installed on this computer.
pub fn installed() -> Vec<&'static str> {
    let home = dirs::home_dir();
    let app_dirs = if cfg!(target_os = "macos") {
        let mut d = vec![PathBuf::from("/Applications")];
        if let Some(h) = &home {
            d.push(h.join("Applications"));
        }
        d
    } else {
        Vec::new()
    };
    let local_appdata = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        None
    };
    let exists = |p: &Path| p.exists();
    let list = |p: &Path| -> Vec<String> {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    detect(&Probe {
        bin_dirs: runner::search_dirs(std::env::var_os("PATH").as_deref(), home.as_deref()),
        app_dirs,
        home,
        local_appdata,
        exists: &exists,
        list: &list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn run(files: &[&str], extensions: &[&str]) -> Vec<&'static str> {
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        let exists = |p: &Path| files.contains(p);
        let exts: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();
        let list = |p: &Path| {
            if p == Path::new("/home/me/.vscode/extensions") {
                exts.clone()
            } else {
                Vec::new()
            }
        };
        detect(&Probe {
            bin_dirs: vec![PathBuf::from("/usr/local/bin")],
            app_dirs: vec![PathBuf::from("/Applications")],
            home: Some(PathBuf::from("/home/me")),
            local_appdata: Some(PathBuf::from("/local")),
            exists: &exists,
            list: &list,
        })
    }

    #[test]
    fn nothing_installed() {
        assert!(run(&[], &[]).is_empty());
    }

    #[test]
    fn finds_clis_apps_and_extensions_in_catalogue_order() {
        let found = run(
            &[
                "/Applications/Cursor.app",
                "/Applications/Claude.app",
                if cfg!(windows) {
                    "/usr/local/bin/claude.exe"
                } else {
                    "/usr/local/bin/claude"
                },
                "/home/me/.opencode/bin/opencode",
                "/local/Programs/Microsoft VS Code",
            ],
            &["saoudrizwan.claude-dev-3.1.0", "ms-python.python-2026.1"],
        );
        assert_eq!(
            found,
            vec![
                "claude-code",
                "claude-desktop",
                "cursor",
                "vscode",
                "opencode",
                "cline"
            ]
        );
    }

    #[test]
    fn extension_prefix_must_match_from_the_start() {
        assert!(run(&[], &["other.continue.continue-1.0"]).is_empty());
        assert_eq!(run(&[], &["continue.continue-1.0.0"]), vec!["continue"]);
    }
}
