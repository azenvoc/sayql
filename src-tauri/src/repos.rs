//! Git repositories an audit reads: SayQL copies each into `repos/<source id>` (mode 700) with
//! the system `git` and the person's own setup (SSH keys, credential helper), or a read-only
//! token they saved for it. Code never goes anywhere but this computer.
//!
//! What's enforced on every git run:
//! - no prompts: `GIT_TERMINAL_PROMPT=0`, ssh in `BatchMode`, no askpass programs;
//! - only `https://`, `ssh://` and `git@host:path` addresses, always after `--`, and only the
//!   https and ssh protocols (`protocol.allow`);
//! - no hooks (`core.hooksPath=/dev/null`), no fsmonitor command, symlinks checked out as plain
//!   files (so a link can't point the audit outside the copy), no LFS downloads;
//! - a token goes in an `http.extraHeader` set through `GIT_CONFIG_COUNT/KEY/VALUE` in git's
//!   environment: never on the command line, never in a file.
//!
//! The copy is SayQL's own: an update is `fetch` then `reset --hard FETCH_HEAD`.

use crate::error::{AppError, AppResult};
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Commits kept in a copy: enough for recent history, cheap for big repositories.
const DEPTH: &str = "50";
const CLONE_TIME: Duration = Duration::from_secs(10 * 60);
const CHECK_TIME: Duration = Duration::from_secs(60);
const LOCAL_TIME: Duration = Duration::from_secs(30);

/// A read-only token and the user name sent with it. No `Debug`: it holds the token.
pub struct Token {
    pub user: String,
    pub secret: String,
}

/// The user name a host expects with a token, for the hosts SayQL knows; others ask for one.
pub fn token_user(url: &str) -> Option<&'static str> {
    let host = host(url)?.to_ascii_lowercase();
    if host == "github.com" || host.ends_with(".github.com") {
        Some("x-access-token")
    } else if host == "gitlab.com" || host.starts_with("gitlab.") {
        Some("oauth2")
    } else if host == "bitbucket.org" {
        Some("x-token-auth")
    } else {
        None
    }
}

/// The host of an address [`check_url`] accepts.
pub fn host(url: &str) -> Option<String> {
    let url = url.trim();
    let authority = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        rest.split('/').next()?
    } else {
        url.strip_prefix("git@")?.split(':').next()?
    };
    let host = authority.rsplit('@').next()?;
    // ssh://host:2222/..., https://host:8443/...
    let host = host.split(':').next()?;
    Some(host.to_string()).filter(|h| !h.is_empty())
}

/// The address, trimmed, if git may fetch it: `https://host/path`, `ssh://[user@]host/path` or
/// `git@host:path`. No passwords or tokens in it (they go in the token field), nothing git
/// could read as an option or another transport.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter the repository's address.".into());
    }
    if url.len() > 2000 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like a repository address.".into());
    }
    #[cfg(test)]
    if url.starts_with("file://") {
        return Ok(url.to_string());
    }
    let ok = if let Some(rest) = url.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or("");
        if authority.contains('@') {
            return Err(
                "Leave the user name and password out of the address: add a token instead.".into(),
            );
        }
        valid_host(authority.split(':').next().unwrap_or("")) && rest.len() > authority.len() + 1
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let authority = rest.split('/').next().unwrap_or("");
        let host = authority.rsplit('@').next().unwrap_or("");
        valid_host(host.split(':').next().unwrap_or("")) && rest.len() > authority.len() + 1
    } else if let Some(rest) = url.strip_prefix("git@") {
        match rest.split_once(':') {
            Some((host, path)) => valid_host(host) && !path.is_empty() && !path.starts_with('-'),
            None => false,
        }
    } else if url.starts_with("http://") {
        return Err(
            "Use the repository's https:// address: SayQL doesn't fetch code unencrypted.".into(),
        );
    } else {
        false
    };
    if ok {
        Ok(url.to_string())
    } else {
        Err("Use the repository's https:// or SSH address (git@host:owner/repo.git), as its Clone button shows it.".into())
    }
}

fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && !h.starts_with(['-', '.'])
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A branch name git can't read as an option or a range; empty is the default branch.
pub fn check_branch(branch: &str) -> Result<String, String> {
    let b = branch.trim();
    let ok = b.is_empty()
        || (b.len() <= 250
            && !b.starts_with(['-', '/', '.'])
            && !b.ends_with(['/', '.'])
            && !b.contains("..")
            && !b.contains("@{")
            && !b.ends_with(".lock")
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-/+".contains(c)));
    if ok {
        Ok(b.to_string())
    } else {
        Err(format!("“{b}” isn't a branch name SayQL can use."))
    }
}

/// A folder inside the repository, relative, without `..`; empty means the whole repository.
pub fn check_subdir(subdir: &str) -> Result<Option<String>, String> {
    let s = subdir.trim().trim_matches('/');
    if s.is_empty() {
        return Ok(None);
    }
    let ok = !s.starts_with('\\')
        && !s.contains(':')
        && s.split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if ok {
        Ok(Some(s.to_string()))
    } else {
        Err("The folder must be a path inside the repository, such as services/api.".into())
    }
}

/// The system `git`, or why there's none.
fn git_bin() -> AppResult<PathBuf> {
    let dirs = crate::audit_runner::search_dirs(
        std::env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    );
    crate::audit_runner::find_in(&["git"], &dirs, |p| p.is_file()).ok_or_else(|| {
        AppError::User(
            "SayQL needs git to read repositories and couldn't find it. Install git (on a Mac, run `xcode-select --install` in a terminal), then try again."
                .into(),
        )
    })
}

/// Config every git run gets, as `GIT_CONFIG_KEY_n`/`VALUE_n` pairs: no hooks or fsmonitor
/// commands, plain files for symlinks, https and ssh only, and the token's header if any.
fn git_config(token: Option<&Token>) -> Vec<(&'static str, String)> {
    let mut config: Vec<(&'static str, String)> = vec![
        ("core.hooksPath", "/dev/null".into()),
        ("core.fsmonitor", "false".into()),
        ("core.symlinks", "false".into()),
        ("protocol.allow", "never".into()),
        ("protocol.https.allow", "always".into()),
        ("protocol.ssh.allow", "always".into()),
    ];
    #[cfg(test)]
    config.push(("protocol.file.allow", "always".into()));
    if let Some(t) = token {
        let basic =
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", t.user, t.secret));
        config.push(("http.extraHeader", format!("Authorization: Basic {basic}")));
    }
    config
}

/// Inherited variables that would point git at another repository, config or prompt.
fn steers_git(name: &str) -> bool {
    name.starts_with("GIT_CONFIG")
        || matches!(
            name,
            "GIT_DIR"
                | "GIT_WORK_TREE"
                | "GIT_INDEX_FILE"
                | "GIT_OBJECT_DIRECTORY"
                | "GIT_ALTERNATE_OBJECT_DIRECTORIES"
                | "GIT_NAMESPACE"
                | "GIT_ASKPASS"
                | "SSH_ASKPASS"
                | "GIT_SSH"
                | "GIT_SSH_COMMAND"
                | "GIT_EXTERNAL_DIFF"
        )
}

/// A git command with SayQL's limits (see the module docs) and the token, if any, in its
/// environment.
fn command(git: &Path, token: Option<&Token>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(git);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(steers_git) {
            cmd.env_remove(name);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("LC_ALL", "C");
    let config = git_config(token);
    cmd.env("GIT_CONFIG_COUNT", config.len().to_string());
    for (i, (key, value)) in config.iter().enumerate() {
        cmd.env(format!("GIT_CONFIG_KEY_{i}"), key)
            .env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::agent_process::no_window(&mut cmd);
    cmd
}

/// Runs git; its stdout, or a message saying what went wrong.
async fn run(
    git: &Path,
    token: Option<&Token>,
    args: &[&str],
    limit: Duration,
    host: &str,
) -> AppResult<String> {
    let mut cmd = command(git, token);
    cmd.args(args);
    let out = match tokio::time::timeout(limit, cmd.output()).await {
        Ok(out) => out.map_err(|e| AppError::User(format!("Could not run git: {e}")))?,
        Err(_) => {
            return Err(AppError::User(format!(
                "git took longer than {} to answer for {host}, and was stopped.",
                if limit.as_secs() >= 60 {
                    format!("{} minutes", limit.as_secs() / 60)
                } else {
                    format!("{} seconds", limit.as_secs())
                }
            )));
        }
    };
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = match token {
        Some(t) if !t.secret.is_empty() => stderr.replace(&t.secret, "…"),
        _ => stderr.into_owned(),
    };
    Err(AppError::User(explain(&stderr, host)))
}

/// git's error in plain words.
pub fn explain(stderr: &str, host: &str) -> String {
    let low = stderr.to_lowercase();
    let has = |w: &str| low.contains(w);
    if has("could not read username")
        || has("terminal prompts disabled")
        || has("authentication failed")
        || has("access denied")
        || has("invalid username or password")
        || has("http basic")
        || has("returned error: 401")
        || has("returned error: 403")
    {
        format!(
            "{host} didn't accept the sign-in. Add a read-only token for this repository, or set up git's credential helper for {host}."
        )
    } else if has("permission denied (publickey") {
        format!(
            "{host} refused your SSH key. Check that `ssh -T git@{host}` works in a terminal, or use the https:// address with a token."
        )
    } else if has("host key verification failed") {
        format!(
            "This computer doesn't know {host}'s SSH host key yet. Connect once in a terminal (`ssh -T git@{host}`), then try again."
        )
    } else if has("remote branch") && has("not found") || has("couldn't find remote ref") {
        "The repository has no such branch. Check its name.".into()
    } else if has("repository not found")
        || has("not found")
        || has("does not appear to be a git repository")
    {
        format!(
            "{host} says there's no such repository, or you can't see it: check the address, or add a token that can read it."
        )
    } else if has("could not resolve host") || has("couldn't resolve host") {
        format!("Couldn't reach {host}. Check the address and your internet connection.")
    } else {
        let last = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("git stopped without saying why.");
        format!("git couldn't read the repository: {last}")
    }
}

/// Whether git can read `branch` (empty: the default branch) of `url` with the person's setup
/// or `token`, without copying anything (`git ls-remote`).
pub async fn check_access(url: &str, branch: &str, token: Option<&Token>) -> AppResult<()> {
    let url = check_url(url)?;
    let branch = check_branch(branch)?;
    let git = git_bin()?;
    let host = host(&url).unwrap_or_else(|| "The server".into());
    let pattern = if branch.is_empty() {
        "HEAD".to_string()
    } else {
        format!("refs/heads/{branch}")
    };
    let out = run(
        &git,
        token,
        &["ls-remote", "--", &url, &pattern],
        CHECK_TIME,
        &host,
    )
    .await?;
    if out.trim().is_empty() {
        return Err(if branch.is_empty() {
            "The repository is empty.".into()
        } else {
            format!("The repository has no branch “{branch}”.").into()
        });
    }
    Ok(())
}

/// Copies `branch` of `url` into `dest`, or brings an earlier copy up to date; the commit now
/// checked out.
pub async fn sync(
    url: &str,
    branch: &str,
    dest: &Path,
    token: Option<&Token>,
) -> AppResult<String> {
    let url = check_url(url)?;
    let branch = check_branch(branch)?;
    let git = git_bin()?;
    let host = host(&url).unwrap_or_else(|| "The server".into());
    let dir = dest.to_string_lossy().into_owned();
    let same_remote = dest.join(".git").is_dir()
        && run(
            &git,
            None,
            &["-C", &dir, "config", "--get", "remote.origin.url"],
            LOCAL_TIME,
            &host,
        )
        .await
        .is_ok_and(|u| u.trim() == url);
    if same_remote {
        let refspec = if branch.is_empty() { "HEAD" } else { &branch };
        run(
            &git,
            token,
            &[
                "-C",
                &dir,
                "fetch",
                "--depth",
                DEPTH,
                "--no-tags",
                "origin",
                refspec,
            ],
            CLONE_TIME,
            &host,
        )
        .await?;
        run(
            &git,
            None,
            &["-C", &dir, "reset", "--quiet", "--hard", "FETCH_HEAD"],
            LOCAL_TIME,
            &host,
        )
        .await?;
    } else {
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        if let Some(parent) = dest.parent() {
            private_dir(parent)?;
        }
        let mut args = vec![
            "clone",
            "--quiet",
            "--depth",
            DEPTH,
            "--single-branch",
            "--no-tags",
        ];
        if !branch.is_empty() {
            args.extend(["--branch", &branch]);
        }
        args.extend(["--", &url, &dir]);
        if let Err(e) = run(&git, token, &args, CLONE_TIME, &host).await {
            let _ = std::fs::remove_dir_all(dest);
            return Err(e);
        }
        private_dir(dest)?;
    }
    head(dest)
        .await
        .ok_or_else(|| "git copied the repository but SayQL couldn't read its commit.".into())
}

/// Creates `dir` if needed, readable by this user only.
pub(crate) fn private_dir(dir: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The commit checked out in `dir`, if it's a git working copy (a local source may not be).
pub async fn head(dir: &Path) -> Option<String> {
    let git = git_bin().ok()?;
    let dir = dir.to_string_lossy();
    let out = run(
        &git,
        None,
        &["-C", &dir, "rev-parse", "--verify", "--quiet", "HEAD"],
        LOCAL_TIME,
        "git",
    )
    .await
    .ok()?;
    let sha = out.trim();
    is_sha(sha).then(|| sha.to_string())
}

fn is_sha(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The files changed between two commits of the copy in `dir` (`git diff --name-only`).
/// Both commits must be in the copy, which keeps the last 50.
#[allow(dead_code, reason = "used by the code map (part C of the 0.5.0 plan)")]
pub async fn changed_files(dir: &Path, old: &str, new: &str) -> AppResult<Vec<String>> {
    if !is_sha(old) || !is_sha(new) {
        return Err("That isn't a commit.".into());
    }
    let git = git_bin()?;
    let dir = dir.to_string_lossy();
    let out = run(
        &git,
        None,
        &[
            "-C",
            &dir,
            "diff",
            "--no-ext-diff",
            "--name-only",
            old,
            new,
            "--",
        ],
        LOCAL_TIME,
        "git",
    )
    .await?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_https_and_ssh_addresses() {
        for ok in [
            "https://github.com/acme/shop.git",
            "https://gitlab.example.com:8443/team/sub/api",
            "ssh://git@bitbucket.org/acme/shop.git",
            "ssh://git.internal:2222/srv/shop.git",
            "git@github.com:acme/shop.git",
            "  git@gitlab.com:acme/shop  ",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "github.com/acme/shop",
            "http://github.com/acme/shop.git",
            "https://user:pass@github.com/acme/shop.git",
            "https://ghp_x@github.com/acme/shop.git",
            "https://github.com",
            "https://-oProxyCommand=x/acme",
            "ssh://-oProxyCommand=touch%20x/acme",
            "git@-oProxyCommand=x:acme/shop",
            "git@github.com:-acme",
            "git@github.com",
            "ext::sh -c touch% /tmp/pwned",
            "--upload-pack=touch /tmp/x",
            "-u",
            "https://github.com/acme/shop.git --upload-pack=x",
            "https://github.com/a\nb",
            "ftp://example.com/x.git",
            "/Users/me/code/shop",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
        assert!(
            check_url("http://x.com/a")
                .unwrap_err()
                .contains("https://")
        );
        assert!(
            check_url("https://u:p@x.com/a")
                .unwrap_err()
                .contains("token")
        );
    }

    #[test]
    fn branches_and_folders() {
        for ok in ["", "main", "release/2026.10", "feature/a-b_c+d"] {
            assert!(check_branch(ok).is_ok(), "{ok}");
        }
        for bad in [
            "-b",
            "--upload-pack=x",
            "a..b",
            "a b",
            "main.lock",
            "@{-1}",
            "/x",
            "x/",
        ] {
            assert!(check_branch(bad).is_err(), "{bad}");
        }
        assert_eq!(
            check_subdir(" /services/api/ ").unwrap().as_deref(),
            Some("services/api")
        );
        assert_eq!(check_subdir("").unwrap(), None);
        for bad in ["..", "a/../../etc", "./x", "C:\\x", "a//b"] {
            assert!(check_subdir(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn token_user_names_by_host() {
        assert_eq!(token_user("https://github.com/a/b"), Some("x-access-token"));
        assert_eq!(token_user("https://gitlab.com/a/b"), Some("oauth2"));
        assert_eq!(token_user("https://gitlab.acme.dev/a/b"), Some("oauth2"));
        assert_eq!(
            token_user("https://bitbucket.org/a/b"),
            Some("x-token-auth")
        );
        assert_eq!(token_user("https://git.acme.dev/a/b"), None);
        assert_eq!(
            host("git@github.com:a/b.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            host("ssh://git@host.dev:2222/a").as_deref(),
            Some("host.dev")
        );
    }

    #[test]
    fn the_token_is_never_on_the_command_line() {
        let token = Token {
            user: "x-access-token".into(),
            secret: "ghp_SECRET123".into(),
        };
        let mut cmd = command(Path::new("/usr/bin/git"), Some(&token));
        cmd.args(["ls-remote", "--", "https://github.com/a/b", "HEAD"]);
        let std = cmd.as_std();
        for a in std.get_args() {
            let a = a.to_string_lossy();
            assert!(!a.contains("SECRET") && !a.contains("Authorization"), "{a}");
        }
        let envs: Vec<(String, String)> = std
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        let get = |k: &str| envs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let count: usize = get("GIT_CONFIG_COUNT").unwrap().parse().unwrap();
        let config: Vec<(&str, &str)> = (0..count)
            .map(|i| {
                (
                    get(&format!("GIT_CONFIG_KEY_{i}")).unwrap(),
                    get(&format!("GIT_CONFIG_VALUE_{i}")).unwrap(),
                )
            })
            .collect();
        let header = config
            .iter()
            .find(|(k, _)| *k == "http.extraHeader")
            .unwrap()
            .1;
        let encoded = header.strip_prefix("Authorization: Basic ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(decoded, b"x-access-token:ghp_SECRET123");
        for want in [
            ("core.hooksPath", "/dev/null"),
            ("core.symlinks", "false"),
            ("protocol.allow", "never"),
            ("protocol.https.allow", "always"),
            ("protocol.ssh.allow", "always"),
        ] {
            assert!(config.contains(&want), "{want:?}");
        }
        assert_eq!(get("GIT_TERMINAL_PROMPT"), Some("0"));
        assert_eq!(get("GIT_SSH_COMMAND"), Some("ssh -o BatchMode=yes"));
        // Without a token there's no header at all.
        let plain = command(Path::new("/usr/bin/git"), None);
        assert!(
            !plain
                .as_std()
                .get_envs()
                .any(|(_, v)| v.is_some_and(|v| v.to_string_lossy().contains("Authorization")))
        );
    }

    #[test]
    fn explains_git_errors() {
        let gh = "github.com";
        assert!(explain("fatal: could not read Username for 'https://github.com': terminal prompts disabled", gh).contains("token"));
        assert!(explain("remote: Repository not found.\nfatal: repository 'https://github.com/a/b/' not found", gh).contains("no such repository"));
        assert!(explain("git@github.com: Permission denied (publickey).", gh).contains("SSH key"));
        assert!(explain("warning: Could not find remote branch nope to clone.\nfatal: Remote branch nope not found in upstream origin", gh).contains("no such branch"));
        assert!(
            explain(
                "fatal: unable to access: Could not resolve host: nope.dev",
                "nope.dev"
            )
            .contains("Couldn't reach nope.dev")
        );
        assert_eq!(
            explain("\nfatal: odd\n\n", gh),
            "git couldn't read the repository: fatal: odd"
        );
    }

    /// git, run plainly, for setting up the test repositories.
    fn sh_git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git installed");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn clones_fetches_and_lists_changed_files() {
        if git_bin().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("sayql-repos-{}", uuid::Uuid::new_v4()));
        let bare = root.join("origin.git");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&bare).unwrap();
        sh_git(&bare, &["init", "--quiet", "--bare"]);
        sh_git(&work, &["init", "--quiet"]);
        std::fs::write(work.join("orders.py"), "PAID = 'p'\n").unwrap();
        // A symlink pointing out of the repository.
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", work.join("hosts")).unwrap();
        sh_git(&work, &["add", "."]);
        sh_git(&work, &["commit", "--quiet", "-m", "first"]);
        let url = format!("file://{}", bare.display());
        sh_git(&work, &["push", "--quiet", &url, "HEAD:refs/heads/main"]);
        let first = sh_git(&work, &["rev-parse", "HEAD"]);

        check_access(&url, "main", None).await.unwrap();
        let e = check_access(&url, "nope", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("no branch “nope”"), "{e}");

        let dest = root.join("app/repos/src_abc");
        let got = sync(&url, "main", &dest, None).await.unwrap();
        assert_eq!(got, first);
        assert!(dest.join("orders.py").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let link = std::fs::symlink_metadata(dest.join("hosts")).unwrap();
            assert!(link.is_file(), "a symlink is checked out as a plain file");
            let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            let mode = std::fs::metadata(dest.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
        }

        std::fs::write(work.join("orders.py"), "PAID = 'paid'\n").unwrap();
        std::fs::write(work.join("refunds.py"), "R = 1\n").unwrap();
        sh_git(&work, &["add", "."]);
        sh_git(&work, &["commit", "--quiet", "-m", "second"]);
        sh_git(&work, &["push", "--quiet", &url, "HEAD:refs/heads/main"]);
        let second = sh_git(&work, &["rev-parse", "HEAD"]);
        // A change made in the copy is thrown away by the update.
        std::fs::write(dest.join("orders.py"), "local edit").unwrap();
        let got = sync(&url, "main", &dest, None).await.unwrap();
        assert_eq!(got, second);
        assert_eq!(
            std::fs::read_to_string(dest.join("orders.py")).unwrap(),
            "PAID = 'paid'\n"
        );
        assert_eq!(head(&dest).await.as_deref(), Some(second.as_str()));

        let mut changed = changed_files(&dest, &first, &second).await.unwrap();
        changed.sort();
        assert_eq!(changed, ["orders.py", "refunds.py"]);
        assert!(changed_files(&dest, "--output=x", &second).await.is_err());

        // The default branch, when none is named.
        let dest2 = root.join("app/repos/src_def");
        assert_eq!(sync(&url, "", &dest2, None).await.unwrap(), second);
        // A folder that isn't a repository has no commit.
        assert_eq!(head(&root).await, None);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Fetches a real public GitHub repository over https with the system git, as "Add
    /// repository" and the audit do. Needs the network.
    /// `SAYQL_SMOKE=1 cargo test --lib real_github -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_github_repository() {
        if std::env::var("SAYQL_SMOKE").is_err() {
            return;
        }
        let url = "https://github.com/octocat/Hello-World.git";
        check_access(url, "master", None).await.unwrap();
        check_access(url, "", None).await.unwrap();
        let e = check_access(url, "no-such-branch", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("no branch"), "{e}");
        let e = check_access(
            "https://github.com/octocat/no-such-repo-sayql.git",
            "",
            None,
        )
        .await
        .unwrap_err()
        .to_string();
        println!("missing repository: {e}");
        assert!(e.starts_with("github.com"), "{e}");
        let dest = std::env::temp_dir().join(format!("sayql-gh-{}/src_gh", uuid::Uuid::new_v4()));
        let first = sync(url, "master", &dest, None).await.unwrap();
        println!("cloned {first}");
        assert!(dest.join("README").is_file());
        // Again: fetch and reset, the same commit.
        assert_eq!(sync(url, "master", &dest, None).await.unwrap(), first);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }
}
