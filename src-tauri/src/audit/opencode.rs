//! The audit without Claude Code: SayQL's built-in OpenCode. When the person has no Claude Code,
//! "Run audit" downloads a pinned OpenCode once (with the ripgrep it searches with), checks both
//! against the SHA-256 pinned here, and runs `opencode run` read-only with the AI provider chosen
//! in Settings → AI provider (SayQL AI credits, Anthropic, OpenAI, Gemini, a local model...).
//!
//! Checked against the OpenCode v1.19.5 source (github.com/anomalyco/opencode, tag v1.19.5):
//!
//! - **Permissions** (`src/permission/index.ts`): the last matching rule wins, and an agent's
//!   rules are OpenCode's defaults, then the config's `permission`, then the agent's own
//!   (`src/agent/agent.ts`). The `sayql-audit` agent starts with `"*": "deny"` and allows only
//!   read (not secrets), glob, grep and SayQL's five MCP tools; a tool whose last rule is a `*`
//!   deny isn't offered to the model at all (`Permission.disabled`). `opencode run` without
//!   `--auto` (or `--yolo`, `--dangerously-skip-permissions`, never passed) rejects every request
//!   that would ask. OpenCode appends `external_directory: allow` for its own tool-output folder,
//!   which lies inside the run's private home.
//! - **Config** (`src/config/config.ts`): merged from auth.json well-knowns, the global config
//!   folder, `OPENCODE_CONFIG`, the project's opencode.json and `.opencode` folders,
//!   `~/.opencode`, `OPENCODE_CONFIG_CONTENT`, an account's org config, the managed folder and
//!   macOS managed preferences, then `OPENCODE_PERMISSION`. The run gets a private, empty home
//!   (HOME, XDG folders, temp), no inherited `OPENCODE_*` variable, the project's config turned
//!   off (`OPENCODE_DISABLE_PROJECT_CONFIG` also leaves out its AGENTS.md), and SayQL's config in
//!   `OPENCODE_CONFIG_CONTENT`. Managed policy can't be turned off, so SayQL refuses to run when
//!   there is one ([`managed_policy`]).
//! - **Network**: model requests go to the chosen provider only (`enabled_providers`); models.dev
//!   isn't fetched (`OPENCODE_DISABLE_MODELS_FETCH`, the model is declared in the config), there
//!   are no update checks, no sharing, no default auth plugins, and the background
//!   `npm install @opencode-ai/plugin` into the config folder is skipped by a stub package.json
//!   ([`prepare_home`]). OpenCode downloads ripgrep when there's none on PATH; SayQL puts its
//!   own pinned copy first on PATH instead.
//! - **Search** (`packages/core/src/ripgrep.ts`): `grep` runs ripgrep with `--no-config
//!   --hidden`, so a ripgrep config can't exclude anything and dotfiles such as `.env` are
//!   searched. On macOS and Linux the `rg` on the run's PATH is a script SayQL writes for the
//!   run ([`rg_wrapper`]) that adds a case-insensitive exclude for every secret file
//!   ([`rg_secret_globs`]) after OpenCode's own globs, where ripgrep gives it precedence. On
//!   Windows, where OpenCode looks for `rg.exe` and a script can't stand in, `grep` is denied;
//!   `read` and `glob` remain.
//! - **Symbolic links**: OpenCode checks a path against the sources as written, without
//!   resolving links, so a link inside a source could lead `read` anywhere. SayQL refuses to run
//!   over a source with a link out of the sources ([`link_out`]).
//! - **Secret files** ([`secrets`]): denied to `read` by name, and since OpenCode's patterns are
//!   case-sensitive outside Windows, also by the shape of every secret-looking file a scan of
//!   the sources finds before the run, in any case ([`secret_paths`]).
//! - **Git** (`packages/core/src/git.ts`, `src/project/project.ts`): in a git repository OpenCode
//!   reads anywhere in the repository, not only the folder it runs in, and writes a project id
//!   into its `.git` folder. The run's `GIT_DIR` names a folder that doesn't exist, so OpenCode
//!   sees plain folders (checked in `real_opencode_stays_read_only`).
//! - **MCP**: OpenCode reports itself to MCP servers as `opencode` ([`APP`] in SayQL's app
//!   list) and names the tools `sayql_<tool>`.

use crate::ai::{Api, KeyNeed, Provider};
use crate::audit::runner::{CREDITS_HAIKU, CREDITS_OPUS, CREDITS_SONNET, Line, SAYQL_TOOLS};
use crate::store::ProviderSettings;
use futures_util::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The audit runner's id, as the window names it.
pub const ID: &str = "opencode";
pub const LABEL: &str = "SayQL's built-in OpenCode";
/// The OpenCode release SayQL runs.
pub const VERSION: &str = "1.19.5";
/// The app OpenCode is in SayQL's app list (its MCP client name is `opencode`).
pub const APP: &str = "OpenCode";
/// The agent SayQL defines for the run, and the provider it names.
const AGENT: &str = "sayql-audit";
const PROVIDER_ID: &str = "sayql-audit";
/// The environment variable the model's API key goes in; the config only names it.
pub const KEY_ENV: &str = "SAYQL_AUDIT_KEY";
/// Steps before the agent stops by itself.
const MAX_STEPS: u32 = 150;
/// How long one SayQL tool call may take (OpenCode's default is 5 seconds).
const MCP_TIMEOUT_MS: u32 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    Zip,
    TarGz,
}

/// A release file SayQL downloads, pinned by size and SHA-256.
#[derive(Debug)]
pub struct Asset {
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub archive: Archive,
    /// The executable inside the archive, by file name.
    pub entry: &'static str,
}

/// What a platform downloads: OpenCode and ripgrep.
#[derive(Debug)]
pub struct Platform {
    /// `std::env::consts::OS` and `ARCH`.
    pub os: &'static str,
    pub arch: &'static str,
    pub opencode: Asset,
    pub ripgrep: Asset,
}

// Sizes and digests as GitHub lists them for each release (2026-11-02); x64 uses the
// `-baseline` builds, which run on CPUs without AVX2.
pub const PLATFORMS: [Platform; 4] = [
    Platform {
        os: "macos",
        arch: "aarch64",
        opencode: Asset {
            url: "https://github.com/anomalyco/opencode/releases/download/v1.19.5/opencode-darwin-arm64.zip",
            bytes: 64_278_924,
            sha256: "c1c9e62521192192ef4c3ed1355ada2144055bddb61682217e997b03c55203c1",
            archive: Archive::Zip,
            entry: "opencode",
        },
        ripgrep: Asset {
            url: "https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/ripgrep-15.1.0-aarch64-apple-darwin.tar.gz",
            bytes: 1_777_930,
            sha256: "378e973289176ca0c6054054ee7f631a065874a352bf43f0fa60ef079b6ba715",
            archive: Archive::TarGz,
            entry: "rg",
        },
    },
    Platform {
        os: "macos",
        arch: "x86_64",
        opencode: Asset {
            url: "https://github.com/anomalyco/opencode/releases/download/v1.19.5/opencode-darwin-x64-baseline.zip",
            bytes: 69_100_890,
            sha256: "e1235374cb7e712cdc15e21ba915a029a5d22312b460606d0783754739706775",
            archive: Archive::Zip,
            entry: "opencode",
        },
        ripgrep: Asset {
            url: "https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/ripgrep-15.1.0-x86_64-apple-darwin.tar.gz",
            bytes: 1_894_127,
            sha256: "64811cb24e77cac3057d6c40b63ac9becf9082eedd54ca411b475b755d334882",
            archive: Archive::TarGz,
            entry: "rg",
        },
    },
    Platform {
        os: "windows",
        arch: "x86_64",
        opencode: Asset {
            url: "https://github.com/anomalyco/opencode/releases/download/v1.19.5/opencode-windows-x64-baseline.zip",
            bytes: 81_738_223,
            sha256: "e4dac5cd38a182a24e7b3e18853f01d08cdb137e50b3bd1397c1a89e54d28c63",
            archive: Archive::Zip,
            entry: "opencode.exe",
        },
        ripgrep: Asset {
            url: "https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/ripgrep-15.1.0-x86_64-pc-windows-msvc.zip",
            bytes: 1_810_687,
            sha256: "124510b94b6baa3380d051fdf4650eaa80a302c876d611e9dba0b2e18d87493a",
            archive: Archive::Zip,
            entry: "rg.exe",
        },
    },
    Platform {
        os: "linux",
        arch: "x86_64",
        opencode: Asset {
            url: "https://github.com/anomalyco/opencode/releases/download/v1.19.5/opencode-linux-x64-baseline.tar.gz",
            bytes: 79_651_821,
            sha256: "17c200fd4b2289954cc01552559f9b6d03ec157b2ddfb5eb54144f491656333d",
            archive: Archive::TarGz,
            entry: "opencode",
        },
        ripgrep: Asset {
            url: "https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/ripgrep-15.1.0-x86_64-unknown-linux-musl.tar.gz",
            bytes: 2_263_077,
            sha256: "1c9297be4a084eea7ecaedf93eb03d058d6faae29bbc57ecdaf5063921491599",
            archive: Archive::TarGz,
            entry: "rg",
        },
    },
];

pub fn platform(os: &str, arch: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.os == os && p.arch == arch)
}

/// This computer's downloads, if SayQL has them for it.
pub fn this_platform() -> Result<&'static Platform, String> {
    platform(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "SayQL's built-in OpenCode isn't available for this computer ({} {}). Install Claude Code to run the audit, or copy the audit prompt into another AI app.",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

impl Platform {
    /// What the first audit downloads, in bytes.
    pub fn download_bytes(&self) -> u64 {
        self.opencode.bytes + self.ripgrep.bytes
    }
}

// ---------------------------------------------------------------------------------------------
// Installing.

/// `<app data>/runners/opencode/1.19.5`.
pub fn install_dir(runners: &Path) -> PathBuf {
    runners.join("opencode").join(VERSION)
}

/// The SHA-256 of each executable, written once both were checked and unpacked.
const HASHES: &str = "sha256.json";

/// An installed OpenCode: the executable, and the folder with it and ripgrep (first on PATH).
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub opencode: PathBuf,
    pub bin_dir: PathBuf,
}

/// `name`, with `.exe` on Windows.
pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of a file, as lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// The installed OpenCode in `dir`, if both executables are there and still match the hashes
/// taken when they were unpacked. Checked before every run.
pub fn installed(dir: &Path) -> Option<Installed> {
    let bin_dir = dir.join("bin");
    let hashes: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(HASHES)).ok()?).ok()?;
    for name in ["opencode", "rg"] {
        let want = hashes.get(name).and_then(Value::as_str)?;
        if sha256_file(&bin_dir.join(exe(name))).ok()? != want {
            return None;
        }
    }
    Some(Installed {
        opencode: bin_dir.join(exe("opencode")),
        bin_dir,
    })
}

/// The installed OpenCode in `dir` by its files alone, without checking their hashes (for
/// showing whether the first run downloads; [`installed`] checks them before a run).
pub fn present(dir: &Path) -> Option<PathBuf> {
    let bin = dir.join("bin");
    let oc = bin.join(exe("opencode"));
    (oc.is_file() && bin.join(exe("rg")).is_file() && dir.join(HASHES).is_file()).then_some(oc)
}

/// The most an unpacked executable may be (OpenCode 1.19.5 is about 180 MB).
const MAX_EXECUTABLE: u64 = 512 * 1024 * 1024;

/// Checks a downloaded `archive` against `asset` (size, then SHA-256) and only then unpacks its
/// executable to `dest`, executable bit set. Returns the executable's SHA-256.
pub fn install_from_archive(archive: &Path, asset: &Asset, dest: &Path) -> Result<String, String> {
    let len = std::fs::metadata(archive)
        .map_err(|e| format!("Couldn't read the download: {e}"))?
        .len();
    let sha = sha256_file(archive).map_err(|e| format!("Couldn't read the download: {e}"))?;
    if len != asset.bytes || sha != asset.sha256 {
        return Err(
            "The download didn't match its checksum, so SayQL didn't use it. Try again.".into(),
        );
    }
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let found = match asset.archive {
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            let mut found = false;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let entry_name = entry.name().map_err(|e| e.to_string())?;
                let name = Path::new(entry_name.as_ref())
                    .file_name()
                    .map(|n| n.to_owned());
                if entry.is_file() && name.as_deref() == Some(std::ffi::OsStr::new(asset.entry)) {
                    copy_limited(&mut entry, &mut out)?;
                    found = true;
                    break;
                }
            }
            found
        }
        Archive::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            let mut found = false;
            for entry in tar.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path().map_err(|e| e.to_string())?.into_owned();
                if entry.header().entry_type().is_file()
                    && path.file_name() == Some(std::ffi::OsStr::new(asset.entry))
                {
                    copy_limited(&mut entry, &mut out)?;
                    found = true;
                    break;
                }
            }
            found
        }
    };
    drop(out);
    if !found {
        let _ = std::fs::remove_file(dest);
        return Err(format!("The download has no {}.", asset.entry));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    sha256_file(dest).map_err(|e| e.to_string())
}

fn copy_limited(from: &mut impl Read, to: &mut std::fs::File) -> Result<(), String> {
    let n = std::io::copy(&mut from.take(MAX_EXECUTABLE + 1), to).map_err(|e| e.to_string())?;
    if n > MAX_EXECUTABLE {
        return Err("The download's executable is larger than expected.".into());
    }
    to.flush().map_err(|e| e.to_string())
}

/// One download at a time, so two audits started together don't both unpack into one folder.
fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Downloads OpenCode and ripgrep for this computer into `install_dir(runners)`, reporting
/// `(received, total)` bytes. Each file is checked before it's unpacked; the folder only
/// appears, by a rename, once both are in it. `stop` cancels. An install already there and
/// intact is used as it is.
pub async fn download(
    http: &reqwest::Client,
    runners: &Path,
    stop: &CancellationToken,
    mut progress: impl FnMut(u64, u64),
) -> Result<Installed, String> {
    let platform = this_platform()?;
    let _guard = install_lock().lock().await;
    let dest = install_dir(runners);
    if let Some(done) = installed_off_thread(&dest).await {
        return Ok(done);
    }
    let parent = dest.parent().ok_or("No folder for OpenCode.")?;
    let work = parent.join(format!(".{VERSION}-{}", uuid::Uuid::new_v4()));
    let result = async {
        std::fs::create_dir_all(work.join("bin")).map_err(|e| e.to_string())?;
        let total = platform.download_bytes();
        let mut received = 0u64;
        let mut hashes = serde_json::Map::new();
        for (name, asset) in [("opencode", &platform.opencode), ("rg", &platform.ripgrep)] {
            let archive = work.join(format!("{name}.download"));
            fetch(
                http,
                asset.url,
                asset.bytes,
                &archive,
                stop,
                TIMEOUTS,
                |n| {
                    received += n;
                    progress(received, total);
                },
            )
            .await?;
            // Hashing and unpacking 80 MB is blocking work: off the async threads.
            let dest_exe = work.join("bin").join(exe(name));
            let from = archive.clone();
            let sha =
                tokio::task::spawn_blocking(move || install_from_archive(&from, asset, &dest_exe))
                    .await
                    .map_err(|e| e.to_string())??;
            let _ = std::fs::remove_file(&archive);
            hashes.insert(name.into(), Value::String(sha));
        }
        std::fs::write(work.join(HASHES), Value::Object(hashes).to_string())
            .map_err(|e| e.to_string())?;
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&work, &dest).map_err(|e| e.to_string())?;
        installed_off_thread(&dest)
            .await
            .ok_or_else(|| "OpenCode didn't install correctly. Try again.".to_string())
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// [`installed`] on a blocking thread: it hashes both executables (about 190 MB).
async fn installed_off_thread(dir: &Path) -> Option<Installed> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || installed(&dir))
        .await
        .ok()
        .flatten()
}

const FAILED: &str = "Couldn't download OpenCode. Check your connection and try again.";

/// How long a download waits for the server to answer, and then for each piece of the file.
pub const TIMEOUTS: (Duration, Duration) = (Duration::from_secs(30), Duration::from_secs(60));

/// Downloads `url` into `to`, at most `limit` bytes, reporting each piece's size. Fails when the
/// server doesn't answer within `timeouts.0` or the file stops coming for `timeouts.1`;
/// `stop` cancels.
pub async fn fetch(
    http: &reqwest::Client,
    url: &str,
    limit: u64,
    to: &Path,
    stop: &CancellationToken,
    timeouts: (Duration, Duration),
    mut on_bytes: impl FnMut(u64),
) -> Result<(), String> {
    let (answer, stall) = timeouts;
    let response = tokio::select! {
        _ = stop.cancelled() => return Err("Download cancelled.".to_string()),
        r = tokio::time::timeout(answer, http.get(url).send()) => r,
    };
    let response = response
        .map_err(|_| FAILED)?
        .and_then(|r| r.error_for_status())
        .map_err(|_| FAILED)?;
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            _ = stop.cancelled() => return Err("Download cancelled.".to_string()),
            next = tokio::time::timeout(stall, stream.next()) => next.map_err(|_| FAILED)?,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|_| FAILED)?;
        got += chunk.len() as u64;
        if got > limit {
            return Err("The download is larger than expected, so SayQL didn't use it.".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        on_bytes(chunk.len() as u64);
    }
    file.flush().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Managed policy.

/// Where an administrator's OpenCode policy lives (`src/config/managed.ts`). OpenCode applies it
/// over every other config, so SayQL can't keep a run read-only under one.
pub fn policy_paths() -> Vec<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/opencode")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into()))
            .join("opencode")
    } else {
        PathBuf::from("/etc/opencode")
    };
    let mut out = vec![dir.join("opencode.json"), dir.join("opencode.jsonc")];
    if cfg!(target_os = "macos") {
        // Per user and for the whole computer; each user's folder is checked, not only this
        // user's name as OpenCode would read it.
        let prefs = Path::new("/Library/Managed Preferences");
        if let Ok(users) = std::fs::read_dir(prefs) {
            for u in users.flatten() {
                out.push(u.path().join("ai.opencode.managed.plist"));
            }
        }
        out.push(prefs.join("ai.opencode.managed.plist"));
    }
    out
}

/// The first of `paths` that exists.
pub fn policy_in(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|p| p.exists())
}

/// Why OpenCode can't run here, when an administrator's policy for it is installed.
pub fn managed_policy() -> Option<String> {
    policy_in(policy_paths()).map(|p| {
        format!(
            "An administrator's OpenCode policy is installed on this computer ({}), and OpenCode applies it over SayQL's read-only settings, so SayQL won't run it. Install Claude Code to run the audit, or copy the audit prompt into another AI app.",
            p.display()
        )
    })
}

// ---------------------------------------------------------------------------------------------
// The model.

/// The model a run uses, from Settings → AI provider. OpenCode speaks to it with one of the
/// provider packages built into it, so nothing is installed for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// "SayQL AI credits", "OpenAI"...
    pub label: String,
    pub npm: &'static str,
    pub base_url: String,
    pub model: String,
    /// The provider needs a key: it goes in the run's environment ([`KEY_ENV`]).
    pub key: KeyNeed,
    pub context: u32,
    pub output: u32,
    /// Paid with SayQL AI credits (their refusals are explained as such).
    pub credits: bool,
}

const CREDIT_MODELS: [&str; 3] = [CREDITS_OPUS, CREDITS_SONNET, CREDITS_HAIKU];

/// The model for `p` with the person's settings for it: their agent model (else the expert or
/// typing model). SayQL AI credits only cover their own models, so another name there is Sonnet.
pub fn model_for(p: &Provider, chosen: &ProviderSettings) -> Result<Model, String> {
    let address = crate::ai::endpoint::address(p, chosen);
    let name = chosen.agent().trim().to_string();
    let credits = p.id == crate::ai::SAYQL_CREDITS;
    let (npm, base_url, model, context, output) = match p.api {
        Api::Plan => {
            return Err(format!(
                "{} runs through Claude Code. Install Claude Code to audit with it, or choose another AI provider in Settings → AI provider.",
                p.label
            ));
        }
        Api::Anthropic => {
            let model = if credits && !CREDIT_MODELS.contains(&name.as_str()) {
                CREDITS_SONNET.to_string()
            } else {
                name
            };
            // @ai-sdk/anthropic posts to `<baseURL>/messages`.
            (
                "@ai-sdk/anthropic",
                format!("{}/v1", address.trim_end_matches('/')),
                model,
                200_000,
                32_000,
            )
        }
        Api::OpenAi => (
            "@ai-sdk/openai-compatible",
            address.trim_end_matches('/').to_string(),
            name,
            128_000,
            8_192,
        ),
    };
    if model.is_empty() {
        return Err(format!(
            "Choose a model for {} in Settings → AI provider to run the audit with it.",
            p.label
        ));
    }
    if base_url.is_empty() || base_url == "/v1" {
        return Err(format!(
            "Add the base URL for {} in Settings → AI provider to run the audit with it.",
            p.label
        ));
    }
    Ok(Model {
        label: p.label.to_string(),
        npm,
        base_url,
        model,
        key: p.key,
        context,
        output,
        credits,
    })
}

// ---------------------------------------------------------------------------------------------
// The config and the environment.

/// A value in an [`ordered`] object: plain JSON, or an object already written in order.
enum J {
    V(Value),
    Raw(String),
}

/// A JSON object whose keys keep the order given: OpenCode reads permission rules in order and
/// the last match wins (serde_json's own map sorts keys).
fn ordered(pairs: &[(&str, J)]) -> String {
    let body: Vec<String> = pairs
        .iter()
        .map(|(k, v)| {
            let v = match v {
                J::V(v) => v.to_string(),
                J::Raw(s) => s.clone(),
            };
            format!("{}:{v}", Value::String((*k).into()))
        })
        .collect();
    format!("{{{}}}", body.join(","))
}

mod secrets;
pub use secrets::{Hidden, rg_secret_globs, secret_files, secret_paths};

/// Tools denied by name too, so a rule merged in before SayQL's can't come after it.
const DENIED: [&str; 14] = [
    "bash",
    "edit",
    "write",
    "apply_patch",
    "patch",
    "webfetch",
    "websearch",
    "codesearch",
    "task",
    "todowrite",
    "skill",
    "lsp",
    "question",
    "doom_loop",
];
/// SayQL's tools the audit doesn't use (see `runner::CLAUDE_DENIED`).
const SAYQL_DENIED: [&str; 5] = [
    "export_csv",
    "add_connection",
    "open_query",
    "open_script",
    "list_scripts",
];

/// Whether the model may search file contents: only where SayQL's ripgrep script keeps secret
/// files out of the search (not on Windows).
pub const GREP: &str = if cfg!(windows) { "deny" } else { "allow" };

/// A source's path as OpenCode may be given it, without a trailing slash: macOS names temp and
/// app folders with or without `/private`, and OpenCode compares paths as given, so both.
fn source_names(source: &Path) -> Vec<String> {
    let d = source
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string();
    let mut out = Vec::new();
    if let Some(bare) = d.strip_prefix("/private/") {
        out.push(format!("/{bare}"));
    }
    out.push(d);
    out
}

/// One permission's rules: `(pattern, action)` in order; a lone `*` is written as the action.
pub type Rules = (String, Vec<(String, &'static str)>);

/// The `sayql-audit` agent's permission rules, in order: everything denied, then reading
/// (secrets aside), searching, the code sources outside the working folder, and SayQL's tools.
/// OpenCode reads them in this order and the last match wins. `hidden` is what the scan of the
/// sources found that looks like a secret in any case ([`secret_paths`]), denied last.
pub fn rules(sources: &[PathBuf], hidden: &[Hidden]) -> Vec<Rules> {
    let all = |action: &'static str| vec![("*".to_string(), action)];
    let mut read = all("allow");
    read.extend(secret_files().into_iter().map(|p| (p, "deny")));
    // As OpenCode's own default: the example file names settings without their values.
    read.push((secrets::READABLE.into(), "allow"));
    read.extend(
        secrets::hidden_rules(sources, hidden)
            .into_iter()
            .map(|p| (p, "deny")),
    );
    let mut external = all("deny");
    for d in sources {
        for name in source_names(d) {
            external.push((format!("{name}/*"), "allow"));
        }
    }
    let mut out: Vec<Rules> = vec![
        ("*".into(), all("deny")),
        ("read".into(), read),
        ("glob".into(), all("allow")),
        ("grep".into(), all(GREP)),
        ("list".into(), all("allow")),
        ("external_directory".into(), external),
    ];
    out.extend(
        SAYQL_TOOLS
            .iter()
            .map(|t| (format!("sayql_{t}"), all("allow"))),
    );
    out.extend(
        SAYQL_DENIED
            .iter()
            .map(|t| (format!("sayql_{t}"), all("deny"))),
    );
    out.extend(DENIED.iter().map(|t| (t.to_string(), all("deny"))));
    out
}

/// [`rules`] as OpenCode's `permission` config, keys in order.
pub fn permission(sources: &[PathBuf], hidden: &[Hidden]) -> String {
    let rules = rules(sources, hidden);
    let pairs: Vec<(&str, J)> = rules
        .iter()
        .map(|(perm, list)| {
            let value = match list.as_slice() {
                [(p, action)] if p == "*" => J::V(json!(action)),
                _ => {
                    let inner: Vec<(&str, J)> = list
                        .iter()
                        .map(|(p, a)| (p.as_str(), J::V(json!(a))))
                        .collect();
                    J::Raw(ordered(&inner))
                }
            };
            (perm.as_str(), value)
        })
        .collect();
    ordered(&pairs)
}

/// The most the deny rules for [`secret_paths`] may add to the config, in bytes. The config
/// goes in one environment variable, and Linux allows 128 KiB for one.
pub const MAX_HIDDEN: usize = 64 * 1024;

/// What the run's config needs.
pub struct RunConfig<'a> {
    pub model: &'a Model,
    /// SayQL's MCP server, next to the app.
    pub sayql_mcp: &'a Path,
    /// The MCP server's environment: the person's real home, so it finds SayQL's files and the
    /// keychain, where OpenCode itself has a private one.
    pub mcp_env: &'a [(String, String)],
    /// Every code source; the first is the working folder.
    pub sources: &'a [PathBuf],
    /// What looks like a secret in the sources, in any case ([`secret_paths`]).
    pub hidden: &'a [Hidden],
}

/// The config SayQL gives OpenCode in `OPENCODE_CONFIG_CONTENT`. It never holds the key: it
/// names [`KEY_ENV`], which OpenCode substitutes. Paths with `{` are refused, since OpenCode
/// would read `{env:...}` or `{file:...}` in them, and sources with `*` or `?`, which would
/// widen the rule that lets OpenCode into them (its patterns have no escape). So are more
/// secret-looking files than the config can deny one by one ([`MAX_HIDDEN`]).
pub fn config_json(c: &RunConfig) -> Result<String, String> {
    for s in c.sources {
        let p = s.to_string_lossy();
        if p.contains(['*', '?']) {
            return Err(format!(
                "SayQL can't run OpenCode over “{p}”: it has a * or ? in it, which OpenCode would read as a wildcard. Rename the folder, or install Claude Code to run the audit."
            ));
        }
    }
    let hidden_bytes: usize = secrets::hidden_rules(c.sources, c.hidden)
        .iter()
        .map(|p| p.len() + 10)
        .sum();
    if hidden_bytes > MAX_HIDDEN {
        let some: Vec<&str> = c.hidden.iter().take(3).map(|h| h.rel.as_str()).collect();
        return Err(format!(
            "The sources have {} files that look like secrets ({}, ...), more than SayQL can keep from OpenCode one by one. Remove them from the sources, or install Claude Code to run the audit.",
            c.hidden.len(),
            some.join(", ")
        ));
    }
    let paths = c
        .sources
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .chain([c.sayql_mcp.to_string_lossy().into_owned()])
        .chain(c.mcp_env.iter().map(|(_, v)| v.clone()));
    for p in paths.chain([c.model.base_url.clone(), c.model.model.clone()]) {
        if p.contains('{') {
            return Err(format!(
                "SayQL can't run OpenCode with “{p}”: it has a {{ in it. Rename the folder, or install Claude Code to run the audit."
            ));
        }
    }
    let m = c.model;
    let mut options = serde_json::Map::new();
    options.insert("baseURL".into(), json!(m.base_url));
    if m.key != KeyNeed::None {
        options.insert("apiKey".into(), json!(format!("{{env:{KEY_ENV}}}")));
    }
    let model = format!("{PROVIDER_ID}/{}", m.model);
    let env: serde_json::Map<String, Value> = c
        .mcp_env
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    let perm = permission(c.sources, c.hidden);
    let agent = ordered(&[
        ("build", J::V(json!({ "disable": true }))),
        ("plan", J::V(json!({ "disable": true }))),
        ("general", J::V(json!({ "disable": true }))),
        ("explore", J::V(json!({ "disable": true }))),
        (
            AGENT,
            J::Raw(ordered(&[
                ("mode", J::V(json!("primary"))),
                (
                    "description",
                    J::V(json!("SayQL's read-only database audit")),
                ),
                ("steps", J::V(json!(MAX_STEPS))),
                ("permission", J::Raw(perm.clone())),
            ])),
        ),
    ]);
    let config = ordered(&[
        ("model", J::V(json!(model))),
        ("small_model", J::V(json!(model))),
        ("default_agent", J::V(json!(AGENT))),
        ("enabled_providers", J::V(json!([PROVIDER_ID]))),
        (
            "provider",
            J::V(json!({ PROVIDER_ID: {
                "npm": m.npm,
                "name": m.label,
                "options": options,
                "models": { m.model.clone(): {
                    "name": m.model,
                    "tool_call": true,
                    "limit": { "context": m.context, "output": m.output },
                } },
            } })),
        ),
        (
            "mcp",
            J::V(json!({ "sayql": {
                "type": "local",
                "command": [c.sayql_mcp.to_string_lossy()],
                "environment": env,
                "enabled": true,
                "timeout": MCP_TIMEOUT_MS,
            } })),
        ),
        ("permission", J::Raw(perm)),
        ("agent", J::Raw(agent)),
        ("share", J::V(json!("disabled"))),
        ("autoupdate", J::V(json!(false))),
        ("snapshot", J::V(json!(false))),
        ("lsp", J::V(json!(false))),
        ("formatter", J::V(json!(false))),
        ("instructions", J::V(json!([]))),
        ("plugin", J::V(json!([]))),
        (
            "experimental",
            J::V(json!({ "continue_loop_on_deny": true, "mcp_timeout": MCP_TIMEOUT_MS })),
        ),
    ]);
    Ok(config)
}

/// Inherited variables the run keeps: finding programs, the language, the system folders on
/// Windows, and a company's proxy and certificates. Everything else (keys for other services,
/// every `OPENCODE_*` setting, telemetry) is left out.
fn kept(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.starts_with("LC_")
        || matches!(
            n.as_str(),
            "PATH"
                | "LANG"
                | "LANGUAGE"
                | "TZ"
                | "TERM"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "PATHEXT"
                | "SYSTEMDRIVE"
                | "PROGRAMFILES"
                | "NUMBER_OF_PROCESSORS"
                | "PROCESSOR_ARCHITECTURE"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "NO_PROXY"
                | "ALL_PROXY"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "NODE_EXTRA_CA_CERTS"
        )
}

/// The run's private folders under `run`: an empty home and temp folder, deleted afterwards.
pub struct Home {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub tmp: PathBuf,
    /// Never created (see `GIT_DIR` in [`child_env`]).
    pub no_git: PathBuf,
    /// SayQL's `rg` script for the run, first on its PATH (not on Windows).
    pub bin: PathBuf,
}

impl Home {
    pub fn new(run: &Path) -> Home {
        let home = run.join("home");
        Home {
            config: home.join(".config"),
            data: home.join(".local").join("share"),
            cache: home.join(".cache"),
            state: home.join(".local").join("state"),
            tmp: run.join("tmp"),
            no_git: run.join("no-git"),
            bin: run.join("bin"),
            home,
        }
    }
}

/// The `rg` OpenCode runs on macOS and Linux: SayQL's ripgrep with an exclude per secret file.
/// The excludes go just before the first `--` (after it come the pattern and the path), or at the
/// end: after OpenCode's own `--glob` options, so they win over an include the model chose
/// (ripgrep applies every `--iglob` after the `--glob`s anyway). `--iglob` ignores case, so
/// `.ENV` and `ID_RSA` are left out as `.env` and `id_rsa` are.
pub fn rg_wrapper(real: &Path) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    let globs: Vec<String> = rg_secret_globs()
        .iter()
        .map(|g| quote(&format!("--iglob=!{g}")))
        .collect();
    let globs = globs.join(" ");
    format!(
        "#!/bin/sh\n\
         # Written by SayQL for one audit: ripgrep that never searches secret files.\n\
         n=$#\n\
         added=0\n\
         for a in \"$@\"; do\n\
         \x20 if [ \"$added\" = 0 ] && [ \"$a\" = \"--\" ]; then\n\
         \x20   set -- \"$@\" {globs}\n\
         \x20   added=1\n\
         \x20 fi\n\
         \x20 set -- \"$@\" \"$a\"\n\
         done\n\
         shift \"$n\"\n\
         if [ \"$added\" = 0 ]; then\n\
         \x20 set -- \"$@\" {globs}\n\
         fi\n\
         exec {} \"$@\"\n",
        quote(&real.to_string_lossy())
    )
}

/// The folder whose `rg` the run uses, first on its PATH: SayQL's script for the run, or on
/// Windows (where `grep` is denied) the downloaded ripgrep itself.
pub fn search_dir(h: &Home, installed: &Installed) -> PathBuf {
    if cfg!(windows) {
        installed.bin_dir.clone()
    } else {
        h.bin.clone()
    }
}

/// Creates the private folders, with a stub package in OpenCode's config folder so it doesn't
/// install `@opencode-ai/plugin` from npm in the background (`core/src/npm.ts` skips a folder
/// whose node_modules exists and whose lock lists what package.json does), and SayQL's `rg`
/// script for `rg` (the downloaded ripgrep).
pub fn prepare_home(h: &Home, rg: &Path) -> std::io::Result<()> {
    for d in [&h.home, &h.data, &h.cache, &h.state, &h.tmp, &h.bin] {
        std::fs::create_dir_all(d)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = h.bin.join("rg");
        std::fs::write(&script, rg_wrapper(rg))?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = rg;
    let oc = h.config.join("opencode");
    std::fs::create_dir_all(oc.join("node_modules"))?;
    let deps = json!({ "@opencode-ai/plugin": VERSION });
    std::fs::write(
        oc.join("package.json"),
        json!({ "dependencies": deps }).to_string(),
    )?;
    std::fs::write(
        oc.join("package-lock.json"),
        json!({ "packages": { "": { "dependencies": deps } } }).to_string(),
    )?;
    Ok(())
}

/// The run's whole environment (the process starts from an empty one): the kept inherited
/// variables, PATH with SayQL's ripgrep first, the private home, OpenCode's switches, the config
/// and the key. No `OPENCODE_*` variable but these. No `Debug`: it holds the key.
pub fn child_env(
    h: &Home,
    folder: &Path,
    bin_dir: &Path,
    config: String,
    key: Option<String>,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    let mut path: Vec<PathBuf> = vec![bin_dir.to_path_buf()];
    for (k, v) in inherited {
        let Some(name) = k.to_str() else { continue };
        if name.eq_ignore_ascii_case("PATH") {
            path.extend(std::env::split_paths(&v));
        } else if kept(name) {
            out.push((k, v));
        }
    }
    let joined = std::env::join_paths(path).unwrap_or_default();
    out.push(("PATH".into(), joined));
    let dirs: [(&str, &Path); 13] = [
        ("HOME", &h.home),
        ("USERPROFILE", &h.home),
        ("APPDATA", &h.config),
        ("LOCALAPPDATA", &h.data),
        ("XDG_CONFIG_HOME", &h.config),
        ("XDG_DATA_HOME", &h.data),
        ("XDG_CACHE_HOME", &h.cache),
        ("XDG_STATE_HOME", &h.state),
        ("TMPDIR", &h.tmp),
        ("TEMP", &h.tmp),
        ("TMP", &h.tmp),
        // `opencode run` takes its folder from PWD before the working directory.
        ("PWD", folder),
        // A git folder that doesn't exist: OpenCode's `git rev-parse` fails, so it treats the
        // sources as plain folders. Otherwise it would take the whole repository a source is in
        // as readable, and write its project id into that repository's .git folder.
        ("GIT_DIR", &h.no_git),
    ];
    for (k, v) in dirs {
        out.push((k.into(), v.as_os_str().to_owned()));
    }
    for k in [
        "OPENCODE_DISABLE_PROJECT_CONFIG",
        "OPENCODE_DISABLE_AUTOUPDATE",
        "OPENCODE_DISABLE_MODELS_FETCH",
        "OPENCODE_DISABLE_DEFAULT_PLUGINS",
        "OPENCODE_DISABLE_CLAUDE_CODE",
        "OPENCODE_DISABLE_EXTERNAL_SKILLS",
        "OPENCODE_DISABLE_LSP_DOWNLOAD",
        "OPENCODE_DISABLE_EMBEDDED_WEB_UI",
        "OPENCODE_PURE",
    ] {
        out.push((k.into(), "1".into()));
    }
    out.push(("OPENCODE_CONFIG_CONTENT".into(), config.into()));
    if let Some(key) = key {
        out.push((KEY_ENV.into(), key.into()));
    }
    out
}

/// `opencode run`'s arguments: JSON events, SayQL's agent and model, the folder, a fixed title
/// (so no model request is spent naming the session). The prompt goes on stdin. Never `--auto`,
/// `--yolo` or `--dangerously-skip-permissions`.
pub fn run_args(model: &Model, folder: &Path) -> Vec<String> {
    vec![
        "run".into(),
        "--pure".into(),
        "--format".into(),
        "json".into(),
        "--agent".into(),
        AGENT.into(),
        "--model".into(),
        format!("{PROVIDER_ID}/{}", model.model),
        "--title".into(),
        "SayQL audit".into(),
        "--dir".into(),
        folder.to_string_lossy().into_owned(),
    ]
}

/// A symbolic link in one of `sources` that leads out of all of them, if there is one: `(link,
/// where it leads)`. OpenCode would follow it. Links that lead to nothing are left alone (nothing
/// to read), as are `.git` folders (OpenCode can't read them).
pub fn link_out(sources: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let roots: Vec<PathBuf> = sources
        .iter()
        .map(|s| s.canonicalize().unwrap_or_else(|_| s.clone()))
        .collect();
    let inside = |p: &Path| roots.iter().any(|r| p.starts_with(r));
    let mut stack: Vec<PathBuf> = sources.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(kind) = e.file_type() else { continue };
            let path = e.path();
            if kind.is_symlink() {
                if let Ok(target) = path.canonicalize()
                    && !inside(&target)
                {
                    return Some((path, target));
                }
            } else if kind.is_dir() && e.file_name() != ".git" {
                stack.push(path);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Reading `opencode run --format json`.

/// Turns OpenCode's JSON events (`src/cli/cmd/run.ts`: one `{type, timestamp, sessionID, ...}`
/// per line; `tool_use` when a tool call finished or failed, `text` when a reply is complete,
/// `error` for the session's errors) into progress lines.
#[derive(Debug, Default)]
pub struct OpenCodeStream {
    sources: Vec<(String, PathBuf)>,
    started: bool,
    pub suggested: u32,
    /// The last complete reply: the summary.
    pub last_text: Option<String>,
    /// The session's errors: message and HTTP status, if any.
    pub errors: Vec<(String, Option<u64>)>,
    pub credits: bool,
    pub label: String,
}

const SHORT: usize = 90;

impl OpenCodeStream {
    pub fn new(sources: Vec<(String, PathBuf)>, model: &Model) -> Self {
        OpenCodeStream {
            sources,
            credits: model.credits,
            label: model.label.clone(),
            ..Default::default()
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<Line> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match v.get("type").and_then(Value::as_str) {
            Some("step_start") if !self.started => {
                self.started = true;
                out.push(Line::step("Started"));
            }
            Some("text") => {
                let text = v
                    .pointer("/part/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !text.trim().is_empty() {
                    self.last_text = Some(text.trim().to_string());
                }
            }
            Some("tool_use") => {
                if let Some(part) = v.get("part") {
                    out.extend(self.tool(part));
                }
            }
            Some("error") => {
                let e = v.get("error").unwrap_or(&Value::Null);
                let message = e
                    .pointer("/data/message")
                    .and_then(Value::as_str)
                    .or_else(|| e.get("message").and_then(Value::as_str))
                    .or_else(|| e.get("name").and_then(Value::as_str))
                    .unwrap_or("unknown error")
                    .to_string();
                let status = e.pointer("/data/statusCode").and_then(Value::as_u64);
                self.errors.push((message, status));
            }
            _ => {}
        }
        out
    }

    fn tool(&mut self, part: &Value) -> Option<Line> {
        let name = part.get("tool").and_then(Value::as_str).unwrap_or("");
        let state = part.get("state").unwrap_or(&Value::Null);
        let ok = state.get("status").and_then(Value::as_str) == Some("completed");
        let input = state.get("input").unwrap_or(&Value::Null);
        let s = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let error = state.get("error").and_then(Value::as_str).unwrap_or("");
        let rel = |p: &str| crate::audit::runner::rel_path(&self.sources, p);
        let what = match name {
            "read" => format!("Reading {}", rel(&s("filePath"))),
            "grep" => {
                let place = match s("path") {
                    p if p.is_empty() => String::new(),
                    p => format!(" in {}", rel(&p)),
                };
                format!("Searching for “{}”{place}", short(&s("pattern"), 60))
            }
            "glob" => format!("Listing files matching {}", short(&s("pattern"), 60)),
            "sayql_list_connections" => "Listing SayQL connections".into(),
            "sayql_list_databases" => "Listing databases".into(),
            "sayql_describe_schema" => match s("search") {
                w if w.is_empty() => "Reading the schema, usage and saved notes".into(),
                w => format!("Reading the schema of tables matching “{}”", short(&w, 60)),
            },
            "sayql_run_query" => format!("Running a read-only query: {}", short(&s("sql"), 70)),
            "sayql_suggest_note" => {
                let text = short(&s("text"), SHORT);
                if !ok {
                    return Some(Line::warn(format!(
                        "SayQL refused a suggestion (“{}”): {}",
                        short(&s("text"), 50),
                        short(error.trim_start_matches("Error: "), 160)
                    )));
                }
                self.suggested += 1;
                let remove = input.get("remove").and_then(Value::as_bool) == Some(true);
                return Some(Line::note(if remove {
                    format!("Suggested removing a note: {text}")
                } else if !s("replaces").is_empty() {
                    format!("Suggested a fix to a note: {text}")
                } else {
                    format!("Suggested a note: {text}")
                }));
            }
            "" => return None,
            other => format!("Using {}", other.trim_start_matches("sayql_")),
        };
        Some(if ok {
            Line::step(what)
        } else if error.contains("unavailable tool") {
            // A tool SayQL didn't give it (write, bash...): OpenCode never offered it.
            Line::warn(format!("Not allowed: {name}"))
        } else if error.contains("prevents you from using") {
            Line::warn(format!("{what}: not allowed"))
        } else {
            Line::warn(format!("{what}: failed ({})", short(error, 120)))
        })
    }

    /// Finished with a reply and no error.
    pub fn succeeded(&self) -> bool {
        self.errors.is_empty() && self.last_text.is_some()
    }

    /// What to tell the person when the run failed.
    pub fn failure(&self, stderr: &str, exit: Option<i32>) -> String {
        let (message, status) = self
            .errors
            .first()
            .map(|(m, s)| (m.as_str(), *s))
            .unwrap_or(("", None));
        let all = format!("{message}\n{stderr}").to_lowercase();
        if self.credits
            && let Some(m) = crate::audit::runner::credits_failure(&all, None)
        {
            return m.into();
        }
        if status == Some(401) || status == Some(403) || all.contains("invalid api key") {
            return format!(
                "{} didn't accept the API key. Check it in Settings → AI provider.",
                self.label
            );
        }
        if status == Some(429) || all.contains("rate limit") {
            return format!(
                "{} is busy right now (rate limited). Try again in a few minutes.",
                self.label
            );
        }
        if all.contains("econnrefused")
            || all.contains("unable to connect")
            || all.contains("connection refused")
        {
            return format!(
                "Couldn't reach {}. If it runs on this computer, check it's started, then try again.",
                self.label
            );
        }
        let detail = [message.trim(), stderr.trim()]
            .into_iter()
            .find(|t| !t.is_empty())
            .map(|t| short(t, 300));
        match (detail, exit) {
            (Some(d), _) => format!("OpenCode stopped with an error: {d}"),
            (None, Some(0)) => "OpenCode finished without a summary.".into(),
            (None, Some(code)) => format!("OpenCode stopped with exit code {code}."),
            (None, None) => "OpenCode stopped unexpectedly.".into(),
        }
    }
}

impl crate::audit::runner::RunStream for OpenCodeStream {
    fn feed(&mut self, line: &str) -> Vec<Line> {
        OpenCodeStream::feed(self, line)
    }
    fn suggested(&self) -> u32 {
        self.suggested
    }
    fn summary(&self) -> Option<String> {
        self.last_text.clone().filter(|_| self.errors.is_empty())
    }
    fn succeeded(&self) -> bool {
        OpenCodeStream::succeeded(self)
    }
    fn failure(&self, stderr: &str, exit: Option<i32>) -> String {
        OpenCodeStream::failure(self, stderr, exit)
    }
}

fn short(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests;
