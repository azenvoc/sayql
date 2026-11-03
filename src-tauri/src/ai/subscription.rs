//! "Your Claude plan": requests run through the person's own installed, unmodified Claude Code
//! (`claude -p`), signed in however they set it up, so they count against their own plan.
//! SayQL never reads or handles the sign-in: it starts the binary and reads its answer.
//!
//! Each request is one `claude -p` with no tools, no MCP servers, no hooks, no slash
//! commands and no saved session, in an empty folder: the model only answers in text. Agent
//! mode uses the text protocol (`agent/text_protocol.rs`) on top.

use super::agent::model::{Block, Msg, Role, Turn, Usage};
use super::{GenRequest, Output, prompt};
use crate::audit::runner::{self, CLAUDE, SETTINGS_JSON};
use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// Claude Code's arguments for one text-only answer. The prompt goes on stdin and the system
/// prompt (with the schema) in a file, so neither is on the command line: that is visible to
/// other processes and, on Windows, limited to 32K characters.
pub fn args(model: &str, system_file: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--restricted",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--max-turns",
        "1",
        "--permission-mode",
        "dontAsk",
        "--settings",
        SETTINGS_JSON,
    ]
    .map(String::from)
    .to_vec();
    if !model.is_empty() {
        args.push("--model".into());
        args.push(model.into());
    }
    args.push("--system-prompt-file".into());
    args.push(system_file.to_string_lossy().into_owned());
    args
}

/// Where Claude Code is installed, or what to do about it.
pub fn locate() -> AppResult<PathBuf> {
    let spec = runner::spec(CLAUDE).expect("Claude Code is in the agent list");
    runner::locate(spec).ok_or_else(|| {
        AppError::User(
            "Claude Code isn't installed on this computer. Install it from claude.com/claude-code \
             and sign in, or pick another way to use AI in Settings."
                .into(),
        )
    })
}

/// What Settings shows after checking Claude Code: installed, recent enough and signed in.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanCheck {
    pub version: String,
    /// "claude.ai" for a Claude plan, "apiKey"/other for an API key or cloud provider.
    pub auth_method: String,
    /// "pro", "max", "team"… when signed in with a plan.
    pub plan: Option<String>,
}

/// Checks that Claude Code is installed, recent enough and signed in. Reads only
/// `claude --version` and `claude auth status`, never the sign-in itself.
pub async fn check() -> AppResult<PlanCheck> {
    let bin = locate()?;
    let run = |args: &'static [&'static str]| {
        let bin = bin.clone();
        async move {
            tokio::process::Command::new(&bin)
                .args(args)
                .env("PATH", runner::child_path(&bin))
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(|e| AppError::User(format!("Couldn't start Claude Code: {e}")))
        }
    };
    let out = run(&["--version"]).await?;
    let version_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if let Some(problem) = runner::version_problem(CLAUDE, runner::parse_version(&version_text)) {
        return Err(AppError::User(problem));
    }
    let out = run(&["auth", "status", "--json"]).await?;
    let status: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    if status["loggedIn"].as_bool() != Some(true) {
        return Err(AppError::User(
            "Claude Code isn't signed in. Run `claude` in a terminal and sign in, then check again."
                .into(),
        ));
    }
    Ok(PlanCheck {
        version: version_text
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string(),
        auth_method: status["authMethod"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        plan: status["subscriptionType"].as_str().map(str::to_string),
    })
}

/// The folder Claude Code runs in: this user's own (not the shared temp folder, where another
/// user could leave a CLAUDE.md), holding only the system prompt files of running requests.
fn work_dir() -> AppResult<PathBuf> {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("sayql").join("claude-plan");
    let fail = |e: std::io::Error| {
        AppError::User(format!("Couldn't create a folder for Claude Code: {e}"))
    };
    std::fs::create_dir_all(&dir).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(fail)?;
    }
    Ok(dir)
}

/// One request's system prompt on disk, readable only by this user; deleted when the request
/// ends, however it ends (a dropped future included).
struct PromptFile(PathBuf);

impl PromptFile {
    fn write(dir: &Path, text: &str) -> AppResult<PromptFile> {
        let path = dir.join(format!("prompt-{}.txt", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options
            .open(&path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| {
                AppError::User(format!("Couldn't prepare the request for Claude Code: {e}"))
            })?;
        Ok(PromptFile(path))
    }
}

impl Drop for PromptFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What one run gave back.
#[derive(Debug, Default)]
pub struct Answer {
    pub text: String,
    pub usage: Usage,
    pub truncated: bool,
}

/// Reads Claude Code's stream-json lines: text as it streams, then the final result.
#[derive(Default)]
pub struct StreamReader {
    answer: Answer,
    /// Text from complete assistant messages, used when no partial chunks arrived.
    whole: String,
    streamed: bool,
    error: Option<String>,
    auth_failed: bool,
    done: bool,
}

impl StreamReader {
    /// Takes one line; returns new answer text, if any.
    pub fn feed(&mut self, line: &str) -> String {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return String::new();
        };
        match v["type"].as_str() {
            Some("stream_event") => {
                let e = &v["event"];
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    let t = e["delta"]["text"].as_str().unwrap_or_default();
                    self.streamed = true;
                    self.answer.text.push_str(t);
                    return t.to_string();
                }
                if e["type"] == "message_delta" && e["delta"]["stop_reason"] == "max_tokens" {
                    self.answer.truncated = true;
                }
            }
            Some("assistant") => {
                if v["error"].as_str() == Some("authentication_failed") {
                    self.auth_failed = true;
                }
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "text" {
                        self.whole
                            .push_str(block["text"].as_str().unwrap_or_default());
                    }
                }
            }
            Some("result") => {
                self.done = true;
                let u = &v["usage"];
                self.answer.usage = Usage {
                    input: u["input_tokens"].as_u64().unwrap_or(0)
                        + u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
                    cached: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
                    output: u["output_tokens"].as_u64().unwrap_or(0),
                };
                if v["is_error"].as_bool() == Some(true) {
                    self.error = Some(
                        v["result"]
                            .as_str()
                            .or(v["subtype"].as_str())
                            .unwrap_or("unknown error")
                            .to_string(),
                    );
                } else if !self.streamed && self.answer.text.is_empty() {
                    let text = v["result"].as_str().map(str::to_string);
                    let text = text.unwrap_or_else(|| std::mem::take(&mut self.whole));
                    self.answer.text = text.clone();
                    return text;
                }
            }
            _ => {}
        }
        String::new()
    }

    /// The answer, or why there is none. `stderr` is what Claude Code printed on failure.
    pub fn finish(self, exit_ok: bool, stderr: &str) -> AppResult<Answer> {
        if self.auth_failed {
            return Err(AppError::User(
                "Claude Code's sign-in wasn't accepted. Run `claude` in a terminal and sign in again."
                    .into(),
            ));
        }
        if let Some(e) = self.error {
            return Err(AppError::User(format!(
                "Claude Code couldn't answer: {}",
                short(&e)
            )));
        }
        if !self.done || !exit_ok {
            let detail = stderr.trim();
            return Err(AppError::User(if detail.is_empty() {
                "Claude Code stopped without an answer.".into()
            } else {
                format!("Claude Code stopped without an answer: {}", short(detail))
            }));
        }
        Ok(self.answer)
    }
}

fn short(s: &str) -> String {
    let s = s.trim();
    match s.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Runs Claude Code once with `system` and `prompt`, passing each piece of text to `on_text`.
async fn ask(
    model: &str,
    system: &str,
    prompt: &str,
    mut on_text: impl FnMut(&str),
) -> AppResult<Answer> {
    let bin = locate()?;
    let dir = work_dir()?;
    let system = PromptFile::write(&dir, system)?;
    let mut child = tokio::process::Command::new(&bin)
        .args(args(model, &system.0))
        .current_dir(&dir)
        .env("PATH", runner::child_path(&bin))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Stopping a request (a new keystroke, Stop) drops this future and ends Claude Code.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AppError::User(format!("Couldn't start Claude Code: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin
        .write_all(prompt.as_bytes())
        .await
        .map_err(|e| AppError::User(format!("Couldn't send the request to Claude Code: {e}")))?;
    drop(stdin);
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });
    let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let mut reader = StreamReader::default();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| AppError::User(format!("Lost Claude Code's answer: {e}")))?
    {
        let text = reader.feed(&line);
        if !text.is_empty() {
            on_text(&text);
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|e| AppError::User(format!("Claude Code didn't finish: {e}")))?;
    let stderr = err_task.await.unwrap_or_default();
    reader.finish(status.success(), &stderr)
}

/// Streams SQL for one request into `out`.
pub(super) async fn run(out: &mut Output<'_>, r: &GenRequest) -> AppResult<Option<u64>> {
    let system = format!(
        "{}\n\n{}",
        prompt::instructions(&r.dialect),
        prompt::schema_block(&r.dialect, &r.schema_text)
    );
    let user = prompt::user_message(&r.question, &r.task, r.skill.as_deref());
    let answer = ask(&r.model, &system, &user, |t| out.text(t)).await?;
    if answer.truncated {
        return Err("The generated query was cut off. Try a simpler request.".into());
    }
    Ok(Some(answer.usage.cached).filter(|c| *c > 0))
}

/// The conversation so far as one prompt; messages are already plain text (text protocol).
pub fn conversation(messages: &[Msg]) -> String {
    if let [only] = messages {
        return super::agent::model::render(only);
    }
    let mut s = String::from(
        "The conversation so far, oldest first. Reply as the assistant to the last message.\n",
    );
    for m in messages {
        let who = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        s.push_str(&format!(
            "\n<{who}>\n{}\n</{who}>\n",
            super::agent::model::render(m)
        ));
    }
    s
}

/// One agent turn. Tool calls travel as text (`text_protocol.rs`), so `messages` are plain text.
pub(crate) async fn turn(
    model: &str,
    system: &str,
    schema: &str,
    messages: &[Msg],
) -> AppResult<Turn> {
    let system = format!("{system}\n\n{schema}");
    let answer = ask(model, &system, &conversation(messages), |_| {}).await?;
    Ok(Turn {
        blocks: vec![Block::Text(answer.text)],
        usage: answer.usage,
        truncated: answer.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).expect(flag);
        &args[i + 1]
    }

    #[test]
    fn runs_without_tools_settings_or_saved_sessions() {
        let args = args("claude-haiku-5-5", Path::new("/x/prompt.txt"));
        assert_eq!(value_after(&args, "--tools"), "");
        assert_eq!(value_after(&args, "--max-turns"), "1");
        assert_eq!(value_after(&args, "--model"), "claude-haiku-5-5");
        assert_eq!(value_after(&args, "--settings"), SETTINGS_JSON);
        // The system prompt replaces Claude Code's own and is the last option.
        assert_eq!(args[args.len() - 2], "--system-prompt-file");
        assert_eq!(args[args.len() - 1], "/x/prompt.txt");
        for flag in [
            "--restricted",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(!args.iter().any(|a| a == "--mcp-config"));
        assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn reads_streamed_text_and_usage() {
        let mut r = StreamReader::default();
        let mut got = String::new();
        for line in [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"SELECT"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":" 1;"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"SELECT 1;"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"SELECT 1;","usage":{"input_tokens":900,"cache_read_input_tokens":40,"output_tokens":7}}"#,
        ] {
            got.push_str(&r.feed(line));
        }
        assert_eq!(got, "SELECT 1;");
        let a = r.finish(true, "").unwrap();
        assert_eq!(a.text, "SELECT 1;");
        assert_eq!(
            a.usage,
            Usage {
                input: 900,
                cached: 40,
                output: 7
            }
        );
    }

    #[test]
    fn uses_the_result_when_nothing_streamed() {
        let mut r = StreamReader::default();
        let got = r.feed(r#"{"type":"result","is_error":false,"result":"SELECT 2;","usage":{}}"#);
        assert_eq!(got, "SELECT 2;");
        assert_eq!(r.finish(true, "").unwrap().text, "SELECT 2;");
    }

    #[test]
    fn explains_failures() {
        let mut r = StreamReader::default();
        r.feed(r#"{"type":"assistant","error":"authentication_failed","message":{"content":[]}}"#);
        r.feed(r#"{"type":"result","is_error":true,"result":"Invalid API key","usage":{}}"#);
        assert!(
            r.finish(false, "")
                .unwrap_err()
                .to_string()
                .contains("sign in again")
        );

        let mut r = StreamReader::default();
        r.feed(r#"{"type":"result","is_error":true,"result":"You've hit your usage limit","usage":{}}"#);
        assert!(
            r.finish(true, "")
                .unwrap_err()
                .to_string()
                .contains("usage limit")
        );

        let r = StreamReader::default();
        let e = r
            .finish(false, "error: unknown option")
            .unwrap_err()
            .to_string();
        assert!(e.contains("unknown option"), "{e}");
    }

    #[test]
    fn a_conversation_becomes_one_prompt() {
        let one = conversation(&[Msg::user("How many orders?")]);
        assert_eq!(one, "How many orders?");
        let many = conversation(&[
            Msg::user("How many orders?"),
            Msg {
                role: Role::Assistant,
                blocks: vec![Block::Text("```json\n{\"tool\":\"run_query\"}\n```".into())],
            },
            Msg::user("Result:\n42"),
        ]);
        assert!(many.contains("<User>\nHow many orders?\n</User>"));
        assert!(many.contains("<Assistant>\n```json"));
        assert!(many.trim_end().ends_with("</User>"));
    }

    /// Real Claude Code, signed in: `SAYQL_TEST_CLAUDE_PLAN=1 cargo test plan_answers -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn plan_answers_with_real_claude_code() {
        if std::env::var("SAYQL_TEST_CLAUDE_PLAN").is_err() {
            return;
        }
        let checked = check().await.unwrap();
        assert!(!checked.version.is_empty());
        let mut streamed = String::new();
        let a = ask(
            "claude-haiku-5-5",
            "Reply with one SQLite statement only, no prose or code fence.",
            "Table t(id int). Count its rows.",
            |t| streamed.push_str(t),
        )
        .await
        .unwrap();
        assert!(a.text.to_uppercase().contains("COUNT"), "{}", a.text);
        assert_eq!(streamed, a.text);
    }
}
