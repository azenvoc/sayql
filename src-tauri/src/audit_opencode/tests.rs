use super::*;
use crate::ai::find_provider;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sayql-oc-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn credits_model() -> Model {
    model_for(
        find_provider(crate::ai::SAYQL_CREDITS).unwrap(),
        &ProviderSettings {
            typing_model: "claude-haiku-5-5".into(),
            expert_model: "claude-sonnet-5-5".into(),
            ..Default::default()
        },
    )
    .unwrap()
}

/// OpenCode's `Wildcard.match` (packages/core/src/util/wildcard.ts): `*` matches anything,
/// slashes included; a trailing " *" may match nothing.
fn wildcard(input: &str, pattern: &str) -> bool {
    fn go(i: &[char], p: &[char]) -> bool {
        match p.first() {
            None => i.is_empty(),
            Some('*') => (0..=i.len()).any(|k| go(&i[k..], &p[1..])),
            Some('?') => !i.is_empty() && go(&i[1..], &p[1..]),
            Some(c) => i.first() == Some(c) && go(&i[1..], &p[1..]),
        }
    }
    let i: Vec<char> = input.replace('\\', "/").chars().collect();
    let p: Vec<char> = pattern.replace('\\', "/").chars().collect();
    go(&i, &p)
}

/// OpenCode's `evaluate` (packages/opencode/src/permission/index.ts): the last matching rule
/// wins; none matching asks, which `opencode run` rejects.
fn evaluate(rules: &[Rules], permission: &str, pattern: &str) -> &'static str {
    rules
        .iter()
        .flat_map(|(perm, list)| list.iter().map(move |(p, a)| (perm, p, *a)))
        .rfind(|(perm, p, _)| wildcard(permission, perm) && wildcard(pattern, p))
        .map(|(_, _, a)| a)
        .unwrap_or("ask")
}

#[test]
fn the_pinned_downloads_are_the_official_releases() {
    assert_eq!(PLATFORMS.len(), 4);
    for p in &PLATFORMS {
        for (a, base) in [
            (
                &p.opencode,
                "https://github.com/anomalyco/opencode/releases/download/v1.19.5/",
            ),
            (
                &p.ripgrep,
                "https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/",
            ),
        ] {
            assert!(a.url.starts_with(base), "{}", a.url);
            assert_eq!(a.sha256.len(), 64);
            assert!(
                a.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert!(a.bytes > 1_000_000);
            let zip = a.url.ends_with(".zip");
            assert_eq!(a.archive == Archive::Zip, zip, "{}", a.url);
        }
        // x64 runs the builds without AVX2.
        if p.arch == "x86_64" {
            assert!(p.opencode.url.contains("-baseline."), "{}", p.opencode.url);
        }
        assert_eq!(p.os == "windows", p.opencode.entry.ends_with(".exe"));
    }
    let mac = platform("macos", "aarch64").unwrap();
    assert_eq!(mac.download_bytes(), 64_278_924 + 1_777_930);
    assert!(platform("linux", "aarch64").is_none());
    assert!(install_dir(Path::new("/data/runners")).ends_with("runners/opencode/1.19.5"));
}

fn zip_with(name: &str, body: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file("README.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"not this one").unwrap();
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn targz_with(path: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        t.append_data(&mut h, path, body).unwrap();
        t.into_inner().unwrap().finish().unwrap();
    }
    out
}

fn asset_for(bytes: &[u8], archive: Archive, entry: &'static str) -> Asset {
    let mut h = Sha256::new();
    h.update(bytes);
    Asset {
        url: "https://example.invalid/x",
        bytes: bytes.len() as u64,
        sha256: Box::leak(hex(&h.finalize()).into_boxed_str()),
        archive,
        entry,
    }
}

#[test]
fn a_download_is_unpacked_only_when_its_checksum_matches() {
    let dir = temp("install");
    for (archive, data, entry) in [
        (
            Archive::Zip,
            zip_with("opencode", b"#!/bin/sh\necho hi\n"),
            "opencode",
        ),
        (
            Archive::TarGz,
            targz_with("ripgrep-15.1.0-x/rg", b"#!/bin/sh\necho hi\n"),
            "rg",
        ),
    ] {
        let file = dir.join("download");
        std::fs::write(&file, &data).unwrap();
        let dest = dir.join(entry);
        let good = asset_for(&data, archive, entry);
        let sha = install_from_archive(&file, &good, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho hi\n");
        assert_eq!(sha, sha256_file(&dest).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "executable");
        }
        std::fs::remove_file(&dest).unwrap();

        // One byte different: refused before anything is unpacked.
        let mut tampered = data.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        std::fs::write(&file, &tampered).unwrap();
        let e = install_from_archive(&file, &good, &dest).unwrap_err();
        assert!(e.contains("checksum"), "{e}");
        assert!(!dest.exists());
        // The right bytes under the wrong pinned digest: refused too.
        std::fs::write(&file, &data).unwrap();
        let wrong = Asset {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..asset_for(&data, archive, entry)
        };
        assert!(install_from_archive(&file, &wrong, &dest).is_err());
        assert!(!dest.exists());
        // An archive without the executable.
        let other = asset_for(&data, archive, "nothere");
        assert!(
            install_from_archive(&file, &other, &dest)
                .unwrap_err()
                .contains("has no")
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_install_is_used_only_while_it_matches_its_hashes() {
    let dir = temp("installed");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join(exe("opencode")), b"oc").unwrap();
    std::fs::write(bin.join(exe("rg")), b"rg").unwrap();
    assert_eq!(installed(&dir), None, "no hashes yet");
    assert_eq!(present(&dir), None);
    let hashes = json!({
        "opencode": sha256_file(&bin.join(exe("opencode"))).unwrap(),
        "rg": sha256_file(&bin.join(exe("rg"))).unwrap(),
    });
    std::fs::write(dir.join(HASHES), hashes.to_string()).unwrap();
    let got = installed(&dir).unwrap();
    assert_eq!(got.opencode, bin.join(exe("opencode")));
    assert_eq!(got.bin_dir, bin);
    assert!(present(&dir).is_some());
    std::fs::write(bin.join(exe("rg")), b"changed").unwrap();
    assert_eq!(installed(&dir), None, "a changed file isn't run");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_administrators_policy_stops_the_run() {
    let dir = temp("policy");
    let json = dir.join("opencode.json");
    let jsonc = dir.join("opencode.jsonc");
    assert_eq!(policy_in([json.clone(), jsonc.clone()]), None);
    std::fs::write(&jsonc, "{}").unwrap();
    assert_eq!(policy_in([json, jsonc.clone()]), Some(jsonc));
    let paths = policy_paths();
    if cfg!(target_os = "macos") {
        assert!(paths.contains(&PathBuf::from(
            "/Library/Application Support/opencode/opencode.json"
        )));
        assert!(paths.contains(&PathBuf::from(
            "/Library/Managed Preferences/ai.opencode.managed.plist"
        )));
    } else if cfg!(windows) {
        assert!(paths[0].ends_with("opencode\\opencode.json"));
    } else {
        assert!(paths.contains(&PathBuf::from("/etc/opencode/opencode.json")));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_agent_may_only_read_search_and_use_sayqls_tools() {
    let src2 = PathBuf::from("/data/repos/src_1");
    let rules = rules(&[PathBuf::from("/Users/me/shop"), src2.clone()]);
    // Everything is denied first, so anything not allowed after is denied.
    assert_eq!(rules[0], ("*".to_string(), vec![("*".to_string(), "deny")]));
    for tool in [
        "bash",
        "edit",
        "write",
        "apply_patch",
        "webfetch",
        "websearch",
        "task",
        "skill",
        "todowrite",
        "lsp",
        "anything_new",
        "sayql_export_csv",
        "sayql_add_connection",
        "sayql_open_query",
        "sayql_open_script",
        "sayql_list_scripts",
    ] {
        assert_eq!(evaluate(&rules, tool, "*"), "deny", "{tool}");
    }
    // Searching contents only where SayQL's ripgrep script keeps secret files out.
    assert_eq!(evaluate(&rules, "grep", "*"), GREP);
    assert_eq!(GREP == "allow", cfg!(unix));
    for tool in [
        "glob",
        "sayql_list_connections",
        "sayql_list_databases",
        "sayql_describe_schema",
        "sayql_suggest_note",
        "sayql_run_query",
    ] {
        assert_eq!(evaluate(&rules, tool, "*"), "allow", "{tool}");
    }
    // Read patterns are paths relative to the working folder.
    for (path, want) in [
        ("src/db/orders.ts", "allow"),
        (".env", "deny"),
        ("api/.env", "deny"),
        (".env.production", "deny"),
        ("config/.env.local", "deny"),
        (".env.example", "allow"),
        ("deploy/server.pem", "deny"),
        ("keys/id_ed25519", "deny"),
        (".git/config", "deny"),
        ("../../data/repos/src_1/.git/config", "deny"),
        ("../../data/repos/src_1/billing.py", "allow"),
    ] {
        assert_eq!(evaluate(&rules, "read", path), want, "{path}");
    }
    // Outside the working folder: only the other sources.
    assert_eq!(
        evaluate(&rules, "external_directory", "/data/repos/src_1/lib/*"),
        "allow"
    );
    assert_eq!(evaluate(&rules, "external_directory", "/etc/*"), "deny");
    assert_eq!(
        evaluate(&rules, "external_directory", "/Users/me/*"),
        "deny"
    );
    assert_eq!(
        evaluate(&rules, "external_directory", "/data/repos/src_10/*"),
        "deny"
    );
    // Both names of a macOS /private folder.
    let mac = super::rules(&[PathBuf::from("/private/var/folders/x/src")]);
    assert_eq!(
        evaluate(&mac, "external_directory", "/var/folders/x/src/a/*"),
        "allow"
    );
    assert_eq!(
        evaluate(&mac, "external_directory", "/private/var/folders/x/src/*"),
        "allow"
    );
}

#[test]
fn the_config_writes_the_rules_in_order() {
    let perm = permission(&[PathBuf::from("/Users/me/shop")]);
    assert!(
        perm.starts_with(r#"{"*":"deny","read":{"*":"allow","*.env":"deny","*.env.*":"deny""#),
        "{perm}"
    );
    let pos = |k: &str| perm.find(k).unwrap_or_else(|| panic!("{k} in {perm}"));
    assert!(pos(r#""*.env.*":"deny""#) < pos(r#""*.env.example":"allow""#));
    assert!(perm.contains(r#""external_directory":{"*":"deny","/Users/me/shop/*":"allow"}"#));
    assert!(perm.contains(r#""bash":"deny""#));
    // The JSON is valid and has every rule.
    let v: Value = serde_json::from_str(&perm).unwrap();
    assert_eq!(v["sayql_suggest_note"], "allow");
    assert_eq!(v["write"], "deny");
}

fn run_config(model: &Model) -> String {
    config_json(&RunConfig {
        model,
        sayql_mcp: Path::new("/Applications/SayQL.app/Contents/MacOS/sayql-mcp"),
        mcp_env: &[("HOME".into(), "/Users/me".into())],
        sources: &[
            PathBuf::from("/Users/me/shop"),
            PathBuf::from("/data/src_1"),
        ],
    })
    .unwrap()
}

#[test]
fn the_config_has_one_read_only_agent_one_provider_and_sayql_only() {
    let model = credits_model();
    let text = run_config(&model);
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["default_agent"], "sayql-audit");
    assert_eq!(v["model"], "sayql-audit/claude-sonnet-5-5");
    assert_eq!(v["small_model"], v["model"]);
    assert_eq!(v["enabled_providers"], json!(["sayql-audit"]));
    for builtin in ["build", "plan", "general", "explore"] {
        assert_eq!(v["agent"][builtin]["disable"], true, "{builtin}");
    }
    let agent = &v["agent"]["sayql-audit"];
    assert_eq!(agent["mode"], "primary");
    assert_eq!(agent["permission"], v["permission"]);
    assert_eq!(agent["permission"]["*"], "deny");
    assert!(text.contains(r#""permission":{"*":"deny","read":{"*":"allow""#));
    let p = &v["provider"]["sayql-audit"];
    assert_eq!(p["npm"], "@ai-sdk/anthropic");
    assert_eq!(p["options"]["baseURL"], "https://sayql.dev/api/ai/v1");
    // The key is only named: OpenCode reads it from the run's environment.
    assert_eq!(p["options"]["apiKey"], "{env:SAYQL_AUDIT_KEY}");
    assert_eq!(p["models"]["claude-sonnet-5-5"]["limit"]["output"], 32_000);
    let mcp = v["mcp"].as_object().unwrap();
    assert_eq!(mcp.len(), 1);
    assert_eq!(
        mcp["sayql"]["command"],
        json!(["/Applications/SayQL.app/Contents/MacOS/sayql-mcp"])
    );
    assert_eq!(mcp["sayql"]["environment"]["HOME"], "/Users/me");
    for (k, want) in [
        ("share", json!("disabled")),
        ("autoupdate", json!(false)),
        ("snapshot", json!(false)),
        ("instructions", json!([])),
        ("plugin", json!([])),
    ] {
        assert_eq!(v[k], want, "{k}");
    }
    assert_eq!(v["experimental"]["continue_loop_on_deny"], true);

    // A local model needs no key, so none is named.
    let local = model_for(
        find_provider("ollama").unwrap(),
        &ProviderSettings {
            typing_model: "qwen3".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let v: Value = serde_json::from_str(&run_config(&local)).unwrap();
    assert!(
        v["provider"]["sayql-audit"]["options"]
            .get("apiKey")
            .is_none()
    );
    assert_eq!(
        v["provider"]["sayql-audit"]["npm"],
        "@ai-sdk/openai-compatible"
    );

    // `{` in a path would be read as `{env:...}` or `{file:...}`.
    let e = config_json(&RunConfig {
        model: &model,
        sayql_mcp: Path::new("/x/sayql-mcp"),
        mcp_env: &[],
        sources: &[PathBuf::from("/Users/me/{file:~/.ssh/id_rsa}")],
    })
    .unwrap_err();
    assert!(e.contains("has a {"), "{e}");
}

#[test]
fn models_come_from_the_ai_provider_settings() {
    let p = |id: &str| find_provider(id).unwrap();
    let with = |agent: &str| ProviderSettings {
        typing_model: agent.into(),
        ..Default::default()
    };
    // Credits cover their own models only.
    let m = model_for(p(crate::ai::SAYQL_CREDITS), &with("claude-opus-5-5")).unwrap();
    assert_eq!(m.model, "claude-opus-5-5");
    assert!(m.credits);
    assert_eq!(
        model_for(p(crate::ai::SAYQL_CREDITS), &with("claude-sonnet-4"))
            .unwrap()
            .model,
        CREDITS_SONNET
    );
    let m = model_for(p("anthropic"), &with("claude-opus-5-5")).unwrap();
    assert_eq!(m.base_url, "https://api.anthropic.com/v1");
    assert!(!m.credits);
    let m = model_for(p("openai"), &with("gpt-5")).unwrap();
    assert_eq!(m.npm, "@ai-sdk/openai-compatible");
    assert_eq!(m.base_url, "https://api.openai.com/v1");
    assert_eq!(m.key, KeyNeed::Required);
    let m = model_for(p("gemini"), &with("gemini-3-pro")).unwrap();
    assert_eq!(
        m.base_url,
        "https://generativelanguage.googleapis.com/v1beta/openai"
    );
    let custom = ProviderSettings {
        base_url: "http://localhost:8000/v1/".into(),
        typing_model: "llama".into(),
        ..Default::default()
    };
    assert_eq!(
        model_for(p("custom"), &custom).unwrap().base_url,
        "http://localhost:8000/v1"
    );
    assert!(
        model_for(p("custom"), &with("llama"))
            .unwrap_err()
            .contains("base URL")
    );
    assert!(
        model_for(p("openai"), &with(""))
            .unwrap_err()
            .contains("Choose a model")
    );
    assert!(
        model_for(p(crate::ai::CLAUDE_PLAN), &with("sonnet"))
            .unwrap_err()
            .contains("Claude Code")
    );
    // The agent model wins over the typing model.
    let s = ProviderSettings {
        typing_model: "gpt-mini".into(),
        agent_model: "gpt-5".into(),
        ..Default::default()
    };
    assert_eq!(model_for(p("openai"), &s).unwrap().model, "gpt-5");
}

#[test]
fn the_run_gets_a_private_home_and_no_inherited_opencode_settings() {
    let run = Path::new("/tmp/run");
    let h = Home::new(run);
    let inherited = [
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/Users/me"),
        ("XDG_CONFIG_HOME", "/Users/me/.config"),
        ("LANG", "en_US.UTF-8"),
        ("HTTPS_PROXY", "http://proxy:3128"),
        ("OPENCODE_PERMISSION", r#"{"bash":"allow"}"#),
        ("OPENCODE_CONFIG", "/Users/me/evil.json"),
        ("OPENCODE_CONFIG_DIR", "/Users/me/.oc"),
        ("OPENCODE_CONFIG_CONTENT", "{}"),
        ("OPENCODE_TEST_MANAGED_CONFIG_DIR", "/tmp/empty"),
        ("opencode_permission", "{}"),
        ("ANTHROPIC_API_KEY", "sk-ant-mine"),
        ("OPENAI_API_KEY", "sk-mine"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let env = child_env(
        &h,
        Path::new("/Users/me/shop"),
        Path::new("/app/runners/opencode/1.19.5/bin"),
        "{\"x\":1}".into(),
        Some("sqlc_secret".into()),
        inherited,
    );
    let get = |k: &str| {
        env.iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.to_string_lossy().into_owned())
    };
    let ours = [
        "OPENCODE_DISABLE_PROJECT_CONFIG",
        "OPENCODE_DISABLE_AUTOUPDATE",
        "OPENCODE_DISABLE_MODELS_FETCH",
        "OPENCODE_DISABLE_DEFAULT_PLUGINS",
        "OPENCODE_DISABLE_CLAUDE_CODE",
        "OPENCODE_DISABLE_EXTERNAL_SKILLS",
        "OPENCODE_DISABLE_LSP_DOWNLOAD",
        "OPENCODE_DISABLE_EMBEDDED_WEB_UI",
        "OPENCODE_PURE",
        "OPENCODE_CONFIG_CONTENT",
    ];
    for (k, _) in &env {
        let k = k.to_string_lossy();
        if k.to_ascii_uppercase().starts_with("OPENCODE_") {
            assert!(ours.contains(&k.as_ref()), "{k} was passed on");
        }
    }
    let mut names: Vec<String> = env
        .iter()
        .map(|(k, _)| k.to_string_lossy().to_uppercase())
        .collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before, "each variable once");
    assert_eq!(get("OPENCODE_CONFIG_CONTENT").as_deref(), Some("{\"x\":1}"));
    assert_eq!(get("OPENCODE_DISABLE_PROJECT_CONFIG").as_deref(), Some("1"));
    assert_eq!(get("OPENCODE_PERMISSION"), None);
    assert_eq!(get("OPENCODE_TEST_MANAGED_CONFIG_DIR"), None);
    assert_eq!(get("ANTHROPIC_API_KEY"), None);
    assert_eq!(get("OPENAI_API_KEY"), None);
    assert_eq!(get("OTEL_EXPORTER_OTLP_ENDPOINT"), None);
    assert_eq!(get("HOME"), Some(h.home.to_string_lossy().into_owned()));
    assert_eq!(get("USERPROFILE"), get("HOME"));
    assert_eq!(
        get("XDG_CONFIG_HOME"),
        Some(h.config.to_string_lossy().into_owned())
    );
    for k in ["TMPDIR", "TEMP", "TMP"] {
        assert_eq!(get(k), Some(h.tmp.to_string_lossy().into_owned()), "{k}");
    }
    assert_eq!(get("PWD").as_deref(), Some("/Users/me/shop"));
    // OpenCode sees no git repository, so it neither widens what it reads nor writes into one.
    assert_eq!(
        get("GIT_DIR"),
        Some(h.no_git.to_string_lossy().into_owned())
    );
    assert!(!h.no_git.exists());
    assert_eq!(get("LANG").as_deref(), Some("en_US.UTF-8"));
    assert_eq!(get("HTTPS_PROXY").as_deref(), Some("http://proxy:3128"));
    let path: Vec<PathBuf> = std::env::split_paths(&OsString::from(get("PATH").unwrap())).collect();
    assert_eq!(
        path[0],
        PathBuf::from("/app/runners/opencode/1.19.5/bin"),
        "SayQL's ripgrep first"
    );
    assert!(path.contains(&PathBuf::from("/usr/bin")));
    // The key is in one variable only, and never in the config.
    assert_eq!(get(KEY_ENV).as_deref(), Some("sqlc_secret"));
    assert_eq!(
        env.iter()
            .filter(|(_, v)| v.to_string_lossy().contains("sqlc_secret"))
            .count(),
        1
    );
    let none = child_env(&h, Path::new("/s"), Path::new("/b"), "{}".into(), None, []);
    assert!(none.iter().all(|(k, _)| k != KEY_ENV));
}

#[test]
fn the_private_home_has_a_stub_so_nothing_is_installed_from_npm() {
    let run = temp("home");
    let h = Home::new(&run);
    prepare_home(&h, Path::new("/app/runners/opencode/1.19.5/bin/rg")).unwrap();
    let oc = h.config.join("opencode");
    assert!(oc.join("node_modules").is_dir());
    let pkg: Value =
        serde_json::from_str(&std::fs::read_to_string(oc.join("package.json")).unwrap()).unwrap();
    let lock: Value =
        serde_json::from_str(&std::fs::read_to_string(oc.join("package-lock.json")).unwrap())
            .unwrap();
    assert_eq!(pkg["dependencies"]["@opencode-ai/plugin"], VERSION);
    assert_eq!(lock["packages"][""]["dependencies"], pkg["dependencies"]);
    for d in [&h.home, &h.data, &h.cache, &h.state, &h.tmp] {
        assert!(d.is_dir());
    }
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn opencode_runs_without_auto_approval_and_the_prompt_stays_off_the_command_line() {
    let model = credits_model();
    let args = run_args(&model, Path::new("/Users/me/shop"));
    assert_eq!(args[0], "run");
    for never in [
        "--auto",
        "--yolo",
        "--dangerously-skip-permissions",
        "--attach",
        "--port",
    ] {
        assert!(!args.iter().any(|a| a == never), "{never}");
    }
    let at = |f: &str| &args[args.iter().position(|a| a == f).unwrap() + 1];
    assert_eq!(at("--format"), "json");
    assert_eq!(at("--agent"), "sayql-audit");
    assert_eq!(at("--model"), "sayql-audit/claude-sonnet-5-5");
    assert_eq!(at("--dir"), "/Users/me/shop");
    assert!(args.iter().any(|a| a == "--pure"));
    assert!(!args.iter().any(|a| a.contains("Audit the database")));
    assert!(!args.iter().any(|a| a.contains("sqlc_")));
}

/// Events as `opencode run --format json` 1.19.5 prints them (from the live test's run).
const SAMPLE: &str = r#"{"type":"step_start","timestamp":1,"sessionID":"ses_1","part":{"type":"step-start","id":"prt_1"}}
{"type":"tool_use","timestamp":2,"sessionID":"ses_1","part":{"type":"tool","tool":"sayql_describe_schema","callID":"c1","state":{"status":"completed","input":{"connection_id":"c1"},"output":"Tables: orders","title":"","metadata":{}}}}
{"type":"tool_use","timestamp":3,"sessionID":"ses_1","part":{"type":"tool","tool":"grep","callID":"c2","state":{"status":"completed","input":{"pattern":"legacy_orders","path":"/Users/me/shop/src"},"output":"","title":"","metadata":{}}}}
{"type":"tool_use","timestamp":4,"sessionID":"ses_1","part":{"type":"tool","tool":"read","callID":"c3","state":{"status":"completed","input":{"filePath":"/Users/me/shop/src/db/orders.ts"},"output":"","title":"","metadata":{}}}}
{"type":"tool_use","timestamp":5,"sessionID":"ses_1","part":{"type":"tool","tool":"read","callID":"c4","state":{"status":"error","input":{"filePath":"/Users/me/shop/.env"},"error":"The user has specified a rule which prevents you from using this specific tool call."}}}
{"type":"tool_use","timestamp":5,"sessionID":"ses_1","part":{"type":"tool","tool":"bash","callID":"c8","state":{"status":"error","input":{"command":"touch x"},"error":"Model tried to call unavailable tool 'invalid'. Available tools: glob, grep, read."}}}
{"type":"tool_use","timestamp":6,"sessionID":"ses_1","part":{"type":"tool","tool":"sayql_suggest_note","callID":"c5","state":{"status":"completed","input":{"connection_id":"c1","text":"legacy_orders is unused; use orders","evidence":"no references"},"output":"Suggested.","title":"","metadata":{}}}}
{"type":"tool_use","timestamp":7,"sessionID":"ses_1","part":{"type":"tool","tool":"sayql_suggest_note","callID":"c6","state":{"status":"completed","input":{"connection_id":"c1","text":"No longer true","replaces":"n_1a2b3c4d","remove":true},"output":"Suggested.","title":"","metadata":{}}}}
{"type":"tool_use","timestamp":8,"sessionID":"ses_1","part":{"type":"tool","tool":"sayql_suggest_note","callID":"c7","state":{"status":"error","input":{"connection_id":"c1","text":"alice@example.com is the admin"},"error":"This looks like a value from a row, not a schema-level fact."}}}
not json
{"type":"step_finish","timestamp":9,"sessionID":"ses_1","part":{"type":"step-finish","reason":"tool-calls"}}
{"type":"text","timestamp":10,"sessionID":"ses_1","part":{"type":"text","text":"I suggested 2 notes.","time":{"start":9,"end":10}}}"#;

#[test]
fn turns_opencode_events_into_progress_lines() {
    let mut s = OpenCodeStream::new(
        vec![(String::new(), PathBuf::from("/Users/me/shop"))],
        &credits_model(),
    );
    let lines: Vec<Line> = SAMPLE.lines().flat_map(|l| s.feed(l)).collect();
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Started",
            "Reading the schema, usage and saved notes",
            "Searching for “legacy_orders” in src",
            "Reading src/db/orders.ts",
            "Reading .env: not allowed",
            "Not allowed: bash",
            "Suggested a note: legacy_orders is unused; use orders",
            "Suggested removing a note: No longer true",
            "SayQL refused a suggestion (“alice@example.com is the admin”): This looks like a value from a row, not a schema-level fact.",
        ]
    );
    assert_eq!(lines[4].kind, "warn");
    assert_eq!(lines[5].kind, "warn");
    assert_eq!(lines[6].kind, "note");
    assert_eq!(s.suggested, 2, "the refused one doesn't count");
    assert!(s.succeeded());
    use crate::audit_runner::RunStream;
    assert_eq!(
        RunStream::summary(&s).as_deref(),
        Some("I suggested 2 notes.")
    );
}

#[test]
fn explains_why_an_opencode_run_failed() {
    let err = |message: &str, status: Option<u64>| {
        let mut s = OpenCodeStream::new(Vec::new(), &credits_model());
        let e = match status {
            Some(code) => json!({"name":"APIError","data":{"message":message,"statusCode":code}}),
            None => json!({"name":"UnknownError","data":{"message":message}}),
        };
        s.feed(&json!({"type":"error","error":e}).to_string());
        assert!(!s.succeeded());
        s
    };
    assert!(
        err(
            "That SayQL AI credit key isn't valid or was turned off.",
            Some(401)
        )
        .failure("", Some(1))
        .contains("didn't accept your credit key")
    );
    assert!(
        err("This SayQL AI credit has run out.", Some(402))
            .failure("", Some(1))
            .contains("run out")
    );
    let mut own = err("Incorrect API key provided", Some(401));
    own.credits = false;
    own.label = "OpenAI".into();
    assert_eq!(
        own.failure("", Some(1)),
        "OpenAI didn't accept the API key. Check it in Settings → AI provider."
    );
    let mut local = err(
        "Unable to connect. Is the computer able to access the url?",
        None,
    );
    local.credits = false;
    local.label = "Ollama".into();
    assert!(local.failure("", Some(1)).contains("Couldn't reach Ollama"));
    assert_eq!(
        err("boom", None).failure("", Some(1)),
        "OpenCode stopped with an error: boom"
    );
    let quiet = OpenCodeStream::new(Vec::new(), &credits_model());
    assert!(!quiet.succeeded(), "no reply is no success");
    assert_eq!(
        quiet.failure("", Some(3)),
        "OpenCode stopped with exit code 3."
    );
}

#[test]
fn every_secret_file_is_kept_out_of_searches() {
    for p in SECRET_FILES {
        if p.contains(".git") {
            continue;
        }
        assert!(RG_SECRET_GLOBS.contains(&p), "{p} isn't excluded from grep");
    }
    assert!(RG_SECRET_GLOBS.contains(&"**/.git/**"));
}

/// Runs SayQL's `rg` script over a stand-in ripgrep that prints its arguments, one per line.
#[cfg(unix)]
#[test]
fn the_rg_script_adds_the_excludes_after_opencodes_globs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp("rg");
    let real = dir.join("real rg's");
    std::fs::write(
        &real,
        "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    let h = Home::new(&dir.join("run"));
    prepare_home(&h, &real).unwrap();
    let rg = h.bin.join("rg");
    let run = |args: &[&str]| -> Vec<String> {
        let out = std::process::Command::new(&rg).args(args).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    };
    let excludes: Vec<String> = RG_SECRET_GLOBS
        .iter()
        .map(|g| format!("--glob=!{g}"))
        .collect();
    // grep: the excludes come after the model's include and before `--`, so they win and the
    // pattern and path stay where they were.
    let got = run(&[
        "--no-config",
        "--json",
        "--hidden",
        "--no-messages",
        "--glob=*.env",
        "--glob=!**/.git/**",
        "--",
        "--glob=x y",
        ".",
    ]);
    let mut want: Vec<String> = [
        "--no-config",
        "--json",
        "--hidden",
        "--no-messages",
        "--glob=*.env",
        "--glob=!**/.git/**",
    ]
    .map(String::from)
    .to_vec();
    want.extend(excludes.iter().cloned());
    want.extend(["--", "--glob=x y", "."].map(String::from));
    assert_eq!(got, want);
    // --files (glob): no `--`, so at the end.
    let got = run(&["--no-config", "--files", "--glob=**/*", "."]);
    assert_eq!(&got[..4], ["--no-config", "--files", "--glob=**/*", "."]);
    assert_eq!(got[4..], excludes[..]);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_sources_is_found() {
    let dir = temp("links").canonicalize().unwrap();
    let src = dir.join("shop");
    let other = dir.join("backend");
    let outside = dir.join("outside");
    for d in [&src, &other, &outside, &src.join("lib/deep")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(outside.join("secret.txt"), "x").unwrap();
    std::fs::write(other.join("billing.py"), "x").unwrap();
    std::os::unix::fs::symlink(src.join("lib"), src.join("lib-again")).unwrap();
    std::os::unix::fs::symlink(other.join("billing.py"), src.join("billing.py")).unwrap();
    std::os::unix::fs::symlink(dir.join("nothing-here"), src.join("broken")).unwrap();
    let sources = [src.clone(), other.clone()];
    assert_eq!(
        link_out(&sources),
        None,
        "links within the sources are fine"
    );
    std::os::unix::fs::symlink(&outside, src.join("lib/deep/escape")).unwrap();
    assert_eq!(
        link_out(&sources),
        Some((src.join("lib/deep/escape"), outside.clone()))
    );
    // A link out from inside `.git` doesn't count: OpenCode can't read there.
    std::fs::remove_file(src.join("lib/deep/escape")).unwrap();
    std::fs::create_dir_all(src.join(".git")).unwrap();
    std::os::unix::fs::symlink(&outside, src.join(".git/escape")).unwrap();
    assert_eq!(link_out(&sources), None);
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------------------------
// Live runs of the real OpenCode, against models served on this computer.

/// The real OpenCode for this computer, installed in a throwaway folder: from archives in
/// `SAYQL_OPENCODE_ARCHIVES` when set (the release files, checked like a download), else
/// downloaded.
fn real_opencode(runners: &Path) -> Installed {
    let platform = this_platform().unwrap();
    if let Some(dir) = std::env::var_os("SAYQL_OPENCODE_ARCHIVES") {
        let dir = PathBuf::from(dir);
        let dest = install_dir(runners);
        let bin = dest.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut hashes = serde_json::Map::new();
        for (name, asset) in [("opencode", &platform.opencode), ("rg", &platform.ripgrep)] {
            let file = dir.join(asset.url.rsplit('/').next().unwrap());
            let sha = install_from_archive(&file, asset, &bin.join(exe(name))).unwrap();
            hashes.insert(name.into(), json!(sha));
        }
        std::fs::write(dest.join(HASHES), Value::Object(hashes).to_string()).unwrap();
        return installed(&dest).unwrap();
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(download(
        &reqwest::Client::new(),
        runners,
        &CancellationToken::new(),
        |_, _| {},
    ))
    .unwrap()
}

/// A scratch SayQL config with one SQLite connection, and the built `sayql-mcp`.
fn scratch_sayql(root: &Path) -> (PathBuf, PathBuf) {
    let config = root.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let store = crate::store::Store::new(config.clone());
    store
        .save_connections(&[crate::db::ConnConfig {
            id: "c1".into(),
            name: "Smoke".into(),
            kind: crate::db::Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: root.join("smoke.sqlite").to_string_lossy().into_owned(),
            ssl: false,
            trust_cert: false,
            production: false,
        }])
        .unwrap();
    let mcp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/sayql-mcp");
    assert!(mcp.exists(), "cargo build --bin sayql-mcp first");
    (config, mcp)
}

/// Runs OpenCode as `start_opencode` does: the same config, environment and arguments, prompt
/// on stdin. Returns stdout, stderr and the exit code.
#[allow(
    clippy::too_many_arguments,
    reason = "test helper mirroring the OpenCode command line"
)]
fn run_opencode(
    oc: &Installed,
    model: &Model,
    key: Option<&str>,
    root: &Path,
    sources: &[PathBuf],
    config_dir: &Path,
    mcp: &Path,
    prompt: &str,
) -> (String, String, Option<i32>) {
    let run = root.join("run");
    let home = Home::new(&run);
    prepare_home(&home, &oc.bin_dir.join(exe("rg"))).unwrap();
    let config = config_json(&RunConfig {
        model,
        sayql_mcp: mcp,
        mcp_env: &[(
            "SAYQL_CONFIG_DIR".into(),
            config_dir.to_string_lossy().into_owned(),
        )],
        sources,
    })
    .unwrap();
    let mut child = std::process::Command::new(&oc.opencode)
        .args(run_args(model, &sources[0]))
        .current_dir(&sources[0])
        .env_clear()
        .envs(child_env(
            &home,
            &sources[0],
            &search_dir(&home, oc),
            config,
            key.map(String::from),
            std::env::vars_os(),
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(prompt.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    // The run's private home stays empty of anything from npm.
    let installed_plugin = home
        .config
        .join("opencode/node_modules/@opencode-ai/plugin")
        .exists();
    assert!(!installed_plugin, "OpenCode installed its plugin package");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A scripted OpenAI-compatible model on 127.0.0.1: the first request with tools gets
/// `calls` (as one streamed reply of tool calls), every later one a closing text. Every
/// request body is kept.
fn scripted_model(
    calls: Vec<(&'static str, Value)>,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
    use std::io::{BufRead, BufReader};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut answered_tools = false;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            let mut first = String::new();
            reader.read_line(&mut first).unwrap_or(0);
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap_or(());
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            log.lock().unwrap().push(v.clone());
            let chunk = |delta: Value, finish: Value| {
                format!(
                    "data: {}\n\n",
                    json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"scripted",
                        "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
                )
            };
            let has_tools = v
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|t| !t.is_empty());
            let mut sse = String::new();
            if has_tools && !answered_tools {
                answered_tools = true;
                let tool_calls: Vec<Value> = calls
                    .iter()
                    .enumerate()
                    .map(|(i, (name, args))| {
                        json!({"index": i, "id": format!("call_{i}"), "type": "function",
                        "function": {"name": name, "arguments": args.to_string()}})
                    })
                    .collect();
                sse.push_str(&chunk(
                    json!({"role":"assistant","tool_calls":tool_calls}),
                    Value::Null,
                ));
                sse.push_str(&chunk(json!({}), json!("tool_calls")));
            } else {
                sse.push_str(&chunk(
                    json!({"role":"assistant","content":"Done: tried everything."}),
                    Value::Null,
                ));
                sse.push_str(&chunk(json!({}), json!("stop")));
            }
            sse.push_str(&format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"scripted","choices":[],
                    "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
            ));
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{sse}",
                sse.len()
            );
            let _ = stream.flush();
        }
    });
    (base, seen)
}

/// Runs the real OpenCode with SayQL's exact config, environment and arguments against a
/// scripted local model that asks, in one turn, to write a file, edit one, run shell commands,
/// fetch a web page, start a subagent, read a .env file and a folder outside the sources, read
/// the second source, and call SayQL's tools. Checks what OpenCode offered the model, what each
/// call got back, and that nothing was written. Needs a built `sayql-mcp`; spends nothing.
/// `SAYQL_SMOKE=1 SAYQL_OPENCODE_ARCHIVES=<folder with the release files> cargo test --lib real_opencode -- --ignored --nocapture`
#[test]
#[ignore]
fn real_opencode_stays_read_only() {
    if std::env::var("SAYQL_SMOKE").is_err() {
        return;
    }
    let root = temp("smoke").canonicalize().unwrap();
    // The working folder is part of a git repository: the rest of it isn't a source.
    let repo = root.join("repo");
    let project = repo.join("shop");
    let sibling = repo.join("sibling");
    let backend = root.join("backend");
    let outside = root.join("outside");
    for d in [&project, &sibling, &backend, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(project.join("orders.ts"), "export const PAID = 'p';\n").unwrap();
    std::fs::write(project.join(".env"), "DB_PASSWORD=walrus-7781\n").unwrap();
    // Secret files grep must not search, each with the same marker.
    for f in [
        "prod.env",
        "config.env",
        ".env.local",
        "deploy.pem",
        "id_rsa",
        ".npmrc",
        "aws-credentials",
    ] {
        std::fs::write(project.join(f), "TOKEN = 'puffin-3307'\n").unwrap();
    }
    std::fs::write(sibling.join("notes.txt"), "MARKER = 'lynx-2210'\n").unwrap();
    std::fs::write(backend.join("billing.py"), "MARKER = 'heron-4512'\n").unwrap();
    std::fs::write(outside.join("secret.txt"), "MARKER = 'otter-9031'\n").unwrap();
    let (config, mcp) = scratch_sayql(&root);
    let oc = real_opencode(&root.join("runners"));
    let pwned = project.join("pwned.txt");
    let pwned_backend = backend.join("pwned.txt");
    let (base, seen) = scripted_model(vec![
        ("write", json!({"filePath": pwned, "content": "x"})),
        (
            "edit",
            json!({"filePath": project.join("orders.ts"), "oldString": "PAID", "newString": "PWNED"}),
        ),
        (
            "apply_patch",
            json!({"patchText": "*** Begin Patch\n*** Add File: pwned.txt\n+x\n*** End Patch"}),
        ),
        (
            "bash",
            json!({"command": format!("touch {}", pwned_backend.display()), "description": "x"}),
        ),
        (
            "webfetch",
            json!({"url": "https://example.com", "format": "text"}),
        ),
        (
            "task",
            json!({"description": "x", "prompt": "write pwned.txt", "subagent_type": "general"}),
        ),
        ("read", json!({"filePath": project.join(".env")})),
        ("read", json!({"filePath": outside.join("secret.txt")})),
        ("read", json!({"filePath": sibling.join("notes.txt")})),
        ("read", json!({"filePath": repo.join(".git/config")})),
        ("read", json!({"filePath": backend.join("billing.py")})),
        ("read", json!({"filePath": project.join("orders.ts")})),
        ("glob", json!({"pattern": "**/*.py", "path": backend})),
        // Searching works, but never in secret files, even when asked for them by name.
        ("grep", json!({"pattern": "PAID|puffin", "path": project})),
        (
            "grep",
            json!({"pattern": "puffin", "path": project, "include": "*.env"}),
        ),
        (
            "grep",
            json!({"pattern": "puffin", "path": project.join("prod.env")}),
        ),
        (
            "grep",
            json!({"pattern": "puffin", "path": project, "include": "{prod.env,id_rsa,.npmrc}"}),
        ),
        ("sayql_list_connections", json!({})),
        (
            "sayql_export_csv",
            json!({"connection_id": "c1", "sql": "SELECT 1"}),
        ),
    ]);
    let model = model_for(
        find_provider("custom").unwrap(),
        &ProviderSettings {
            base_url: base,
            typing_model: "scripted".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let (stdout, stderr, code) = run_opencode(
        &oc,
        &model,
        None,
        &root,
        &[project.clone(), backend.clone()],
        &config,
        &mcp,
        "Audit this.",
    );
    let mut stream = OpenCodeStream::new(
        vec![
            ("shop".into(), project.clone()),
            ("backend".into(), backend.clone()),
        ],
        &model,
    );
    for line in stdout.lines() {
        for l in stream.feed(line) {
            println!("{}: {}", l.kind, l.text);
        }
    }
    println!("exit: {code:?}\nstderr: {stderr}");
    let seen = seen.lock().unwrap().clone();
    let with_tools: Vec<&Value> = seen
        .iter()
        .filter(|v| v.get("tools").and_then(Value::as_array).is_some())
        .collect();
    assert!(
        !with_tools.is_empty(),
        "OpenCode never asked the model: {stdout}"
    );
    // What OpenCode offered the model: reading, searching and SayQL's five tools.
    let mut offered: Vec<String> = with_tools[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap_or("").to_string())
        .collect();
    offered.sort();
    println!("offered: {offered:?}");
    for t in &offered {
        assert!(
            ["read", "glob", "grep", "invalid"].contains(&t.as_str())
                || SAYQL_TOOLS.iter().any(|s| *t == format!("sayql_{s}")),
            "{t} was offered"
        );
    }
    for t in SAYQL_TOOLS {
        assert!(offered.contains(&format!("sayql_{t}")), "sayql_{t} missing");
    }
    // What each call got back, from the model's next request.
    let results: Vec<String> = seen
        .iter()
        .flat_map(|v| v["messages"].as_array().cloned().unwrap_or_default())
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].to_string())
        .collect();
    for r in &results {
        println!("result: {}", r.chars().take(240).collect::<String>());
    }
    let all = results.join("\n");
    assert!(
        all.contains("heron-4512"),
        "the second source wasn't readable"
    );
    assert!(all.contains("PAID"), "the working folder wasn't readable");
    assert!(all.contains("Smoke"), "SayQL's tools didn't answer");
    assert!(!all.contains("walrus-7781"), "a .env file was read");
    if cfg!(unix) {
        assert!(
            all.contains("Line 1: export const PAID"),
            "grep didn't search the sources"
        );
    }
    assert!(!all.contains("puffin-3307"), "grep searched a secret file");
    assert!(
        !all.contains("otter-9031"),
        "a folder outside the sources was read"
    );
    assert!(
        !pwned.exists() && !pwned_backend.exists(),
        "a file was written"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("orders.ts")).unwrap(),
        "export const PAID = 'p';\n",
        "a file was edited"
    );
    assert!(!root.join("pwned.txt").exists());
    assert!(
        !all.contains("lynx-2210"),
        "the rest of the repository was read"
    );
    assert!(!all.contains("repositoryformatversion"), ".git was read");
    // OpenCode keeps a project id in a repository's .git folder: not in this one.
    assert!(
        !repo.join(".git/opencode").exists(),
        "OpenCode wrote into .git"
    );
    // SayQL's MCP server logged the run as OpenCode.
    let activity = crate::store::Store::new(config).app_activity(50).unwrap();
    assert!(activity.iter().any(|a| a.app == APP), "{activity:?}");
    let _ = std::fs::remove_dir_all(root);
}

/// Checks what OpenCode does with a symbolic link inside a source that leads out of it: it
/// follows it (its folder check doesn't resolve links), which is why SayQL refuses to run over
/// such a source ([`link_out`], checked here too). `SAYQL_SMOKE=1 SAYQL_OPENCODE_ARCHIVES=<folder>
/// cargo test --lib opencode_follows_links -- --ignored --nocapture`
#[cfg(unix)]
#[test]
#[ignore]
fn opencode_follows_links_so_sayql_refuses_them() {
    if std::env::var("SAYQL_SMOKE").is_err() {
        return;
    }
    let root = temp("links").canonicalize().unwrap();
    let project = root.join("shop");
    let outside = root.join("outside");
    for d in [&project, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(project.join("orders.ts"), "export const PAID = 'p';\n").unwrap();
    std::fs::write(outside.join("secret.txt"), "MARKER = 'otter-9031'\n").unwrap();
    std::os::unix::fs::symlink(&outside, project.join("vendor")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), project.join("notes.txt")).unwrap();
    // SayQL won't start over this source, and names the link.
    let found = link_out(std::slice::from_ref(&project)).expect("the link is found");
    println!(
        "SayQL refuses: {} -> {}",
        found.0.display(),
        found.1.display()
    );
    assert!(found.0 == project.join("vendor") || found.0 == project.join("notes.txt"));
    assert!(found.1.starts_with(root.join("outside")));

    // What OpenCode itself would do with it.
    let (config, mcp) = scratch_sayql(&root);
    let oc = real_opencode(&root.join("runners"));
    let (base, seen) = scripted_model(vec![
        (
            "read",
            json!({"filePath": project.join("vendor/secret.txt")}),
        ),
        ("read", json!({"filePath": project.join("notes.txt")})),
        ("grep", json!({"pattern": "otter", "path": project})),
    ]);
    let model = model_for(
        find_provider("custom").unwrap(),
        &ProviderSettings {
            base_url: base,
            typing_model: "scripted".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let (stdout, stderr, code) = run_opencode(
        &oc,
        &model,
        None,
        &root,
        std::slice::from_ref(&project),
        &config,
        &mcp,
        "Audit this.",
    );
    let mut stream = OpenCodeStream::new(vec![(String::new(), project.clone())], &model);
    for line in stdout.lines() {
        for l in stream.feed(line) {
            println!("{}: {}", l.kind, l.text);
        }
    }
    println!("exit: {code:?}\nstderr: {stderr}");
    let results: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .flat_map(|v| v["messages"].as_array().cloned().unwrap_or_default())
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].to_string())
        .collect();
    for r in &results {
        println!("result: {}", r.chars().take(200).collect::<String>());
    }
    let read_through = results
        .iter()
        .take(2)
        .filter(|r| r.contains("otter-9031"))
        .count();
    println!("reads that followed the link out: {read_through} of 2");
    assert!(
        read_through > 0,
        "OpenCode no longer follows links out of a source: link_out may be unneeded"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Runs the real OpenCode paid with SayQL AI credits against the credits service served locally
/// (scripts/ai-credits/harness.mjs: the real site/api/ai handlers, a fake Anthropic and a fake
/// credit-key check). Checks OpenCode's Anthropic requests are ones the service accepts.
/// `SAYQL_SMOKE=1 SAYQL_OPENCODE_ARCHIVES=<folder> cargo test --lib opencode_credits -- --ignored --nocapture`
#[test]
#[ignore]
fn opencode_credits_run_reaches_the_credits_service() {
    use std::io::BufRead;
    if std::env::var("SAYQL_SMOKE").is_err() {
        return;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut harness = std::process::Command::new("node")
        .arg(repo.join("scripts/ai-credits/harness.mjs"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("node installed");
    let mut first = String::new();
    std::io::BufReader::new(harness.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let started: Value = serde_json::from_str(&first).unwrap();
    let base = started["base"].as_str().unwrap().to_string();
    let log = PathBuf::from(started["log"].as_str().unwrap());

    let root = temp("credits").canonicalize().unwrap();
    let project = root.join("shop");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("orders.ts"), "export const PAID = 'p';\n").unwrap();
    let (config, mcp) = scratch_sayql(&root);
    let oc = real_opencode(&root.join("runners"));
    let model = model_for(
        find_provider(crate::ai::SAYQL_CREDITS).unwrap(),
        &ProviderSettings {
            base_url: base,
            typing_model: "claude-sonnet-5-5".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let (stdout, stderr, code) = run_opencode(
        &oc,
        &model,
        Some("sqlc_test_good"),
        &root,
        std::slice::from_ref(&project),
        &config,
        &mcp,
        "Say hi in one line.",
    );
    let _ = harness.kill();
    let _ = harness.wait();
    let mut stream = OpenCodeStream::new(vec![(String::new(), project.clone())], &model);
    for line in stdout.lines() {
        stream.feed(line);
    }
    println!(
        "exit: {code:?}\nstderr: {stderr}\nerrors: {:?}",
        stream.errors
    );
    let seen: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for e in &seen {
        println!(
            "{} {} {}",
            if e["front"] == true {
                "client →"
            } else {
                "upstream ←"
            },
            e["method"],
            e["path"]
        );
    }
    assert_eq!(
        stream.last_text.as_deref(),
        Some("Done: nothing to suggest.")
    );
    assert!(stream.succeeded(), "{:?}", stream.errors);
    let client: Vec<&Value> = seen.iter().filter(|e| e["front"] == true).collect();
    assert!(!client.is_empty());
    for e in &client {
        assert_eq!(e["path"], "/api/ai/v1/messages");
        assert_eq!(e["auth"], "x-api-key");
    }
    let upstream: Vec<&Value> = seen
        .iter()
        .filter(|e| e["front"] != true && e["front404"] != true)
        .collect();
    assert_eq!(
        upstream.len(),
        client.len(),
        "the service refused a request"
    );
    for e in &upstream {
        assert_eq!(e["body"]["model"], "claude-sonnet-5-5");
    }
    assert!(!stderr.contains("sqlc_test_good"), "the key was printed");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(log);
}
