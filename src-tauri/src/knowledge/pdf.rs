//! Reading a PDF's text in a child process. `pdf-extract` panics on PDFs it doesn't expect (an
//! unknown font encoding, a CJK font with a predefined CMap) and inflates streams without a
//! limit. Release builds abort on a panic, so reading a PDF in SayQL's own process would close
//! the app, and again at every audit that reads the source. Instead SayQL runs its own
//! executable with [`FLAG`]: the PDF goes in on stdin, its text comes out on stdout, and a
//! crash, a hang or a flood of output fails only that file.

use super::MAX_FILE_BYTES;
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The argument that runs SayQL as a PDF reader instead of the app (see `main.rs`).
pub const FLAG: &str = "--extract-pdf";
/// How long one PDF may take.
const TIME: Duration = Duration::from_secs(30);
/// The most text read back from one PDF.
const MAX_TEXT_BYTES: usize = 4 * MAX_FILE_BYTES as usize;
/// The most of the reader's error output kept.
const MAX_ERROR_BYTES: usize = 4096;
/// The memory the reader may use, where the system enforces it (Linux).
#[cfg(unix)]
const MAX_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
/// The reader's exit code when the PDF can't be read (rather than a crash).
const UNREADABLE: i32 = 1;

/// The PDF reader's side: reads a PDF from stdin, writes its text to stdout. The exit code is
/// 0 when it worked, [`UNREADABLE`] with the reason on stderr when the PDF can't be read, and
/// anything else (a panic or an abort) when `pdf-extract` gave up on it.
pub fn reader_main() -> i32 {
    limit_memory();
    let mut bytes = Vec::new();
    let read = std::io::stdin()
        .lock()
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes);
    let result = match read {
        Err(e) => Err(format!("couldn't read the file: {e}")),
        Ok(_) if bytes.len() as u64 > MAX_FILE_BYTES => Err("the file is too large".to_string()),
        Ok(_) => text_of(&bytes),
    };
    match result {
        Ok(text) => {
            let mut out = std::io::stdout().lock();
            match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => 0,
                Err(_) => UNREADABLE,
            }
        }
        Err(e) => {
            let _ = std::io::stderr().lock().write_all(e.as_bytes());
            UNREADABLE
        }
    }
}

/// The text of a PDF, read in this process. Only the reader calls it: it can panic.
fn text_of(bytes: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| e.to_string())
}

/// Caps the reader's memory, so a PDF that inflates without end fails instead of filling the
/// machine. Linux enforces `RLIMIT_DATA` on all of a process's private memory (since 4.7);
/// macOS refuses or ignores it, so there the time limit is the only bound. Windows has no
/// such limit for a single process without a job object.
#[cfg(unix)]
fn limit_memory() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let old = getrlimit(Resource::Data);
    let cap = old
        .maximum
        .map_or(MAX_MEMORY_BYTES, |m| m.min(MAX_MEMORY_BYTES));
    let _ = setrlimit(
        Resource::Data,
        Rlimit {
            current: Some(cap),
            maximum: Some(cap),
        },
    );
}

#[cfg(not(unix))]
fn limit_memory() {}

/// The text of the PDF `name`, read by SayQL's own executable in a child process.
pub fn text(name: &str, bytes: &[u8]) -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Couldn't start SayQL's PDF reader for {name}: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.arg(FLAG);
    text_from(cmd, name, bytes)
}

/// The text the reader `cmd` gives for the PDF `name`, or why there is none.
fn text_from(cmd: Command, name: &str, bytes: &[u8]) -> Result<String, String> {
    let limits = Limits {
        time: TIME,
        stdout: MAX_TEXT_BYTES,
        stderr: MAX_ERROR_BYTES,
    };
    match run(cmd, bytes.to_vec(), &limits) {
        Ok(out) => Ok(String::from_utf8_lossy(&out).into_owned()),
        Err(Failure::Start(e)) => Err(format!("Couldn't start SayQL's PDF reader for {name}: {e}")),
        Err(Failure::Exit { code, stderr }) => {
            let reason = String::from_utf8_lossy(&stderr);
            let reason = reason.lines().next().unwrap_or("").trim();
            if code == Some(UNREADABLE) && !reason.is_empty() {
                Err(format!("{name} isn't a PDF SayQL can read ({reason})."))
            } else {
                Err(format!("{name} isn't a PDF SayQL can read."))
            }
        }
        Err(Failure::TimedOut) => Err(format!(
            "{name} took over {} seconds to read, so SayQL stopped. Try a smaller PDF.",
            limits.time.as_secs()
        )),
        Err(Failure::TooMuchOutput) => Err(format!(
            "{name} has more than {} MB of text. Try a smaller PDF.",
            limits.stdout / 1024 / 1024
        )),
    }
}

/// What a child process may use.
struct Limits {
    /// Wall-clock time, after which it's killed.
    time: Duration,
    /// Bytes read from stdout; one more and it's killed.
    stdout: usize,
    /// Bytes of stderr kept; the rest is read and dropped.
    stderr: usize,
}

/// Why a child process gave no output.
#[derive(Debug)]
enum Failure {
    /// It couldn't be started.
    Start(String),
    /// It exited unsuccessfully or was killed by a signal (`code` is `None` then).
    Exit { code: Option<i32>, stderr: Vec<u8> },
    /// It ran past [`Limits::time`] and was killed.
    TimedOut,
    /// It wrote more than [`Limits::stdout`] and was killed.
    TooMuchOutput,
}

/// Runs `cmd` with `input` on its stdin: its stdout when it exits successfully within `limits`.
fn run(mut cmd: Command, input: Vec<u8>, limits: &Limits) -> Result<Vec<u8>, Failure> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| Failure::Start(e.to_string()))?;
    let deadline = Instant::now() + limits.time;

    // Threads feed stdin and drain stdout and stderr, so a full pipe never blocks either side.
    let stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            // A reader that exits early closes the pipe; its exit status says what happened.
            let _ = stdin.write_all(&input);
        }
    });
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take();
    let (cap, flag) = (limits.stdout, overflow.clone());
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(out) = stdout {
            let _ = out.take(cap as u64 + 1).read_to_end(&mut buf);
        }
        if buf.len() > cap {
            flag.store(true, Ordering::SeqCst);
        }
        buf
    });
    let stderr = child.stderr.take();
    let err_cap = limits.stderr;
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr {
            let _ = err.by_ref().take(err_cap as u64).read_to_end(&mut buf);
            let _ = std::io::copy(&mut err, &mut std::io::sink());
        }
        buf
    });

    let ended = wait(&mut child, deadline, &overflow);
    if ended.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = writer.join();
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    match ended {
        None if overflow.load(Ordering::SeqCst) => Err(Failure::TooMuchOutput),
        None => Err(Failure::TimedOut),
        Some(_) if out.len() > limits.stdout => Err(Failure::TooMuchOutput),
        Some(status) if status.success() => Ok(out),
        Some(status) => Err(Failure::Exit {
            code: status.code(),
            stderr: err,
        }),
    }
}

/// Waits for `child` to exit: its status, or `None` once `deadline` passes or its output
/// overflows (it's still running then).
fn wait(
    child: &mut std::process::Child,
    deadline: Instant,
    overflow: &AtomicBool,
) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if overflow.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Keeps a console window from flashing up for the reader on Windows.
#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF with `text` in a Type 1 font whose `/Encoding` is `encoding`, its
    /// cross-reference table worked out here.
    fn pdf_with_encoding(text: &str, encoding: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /{encoding} >>"),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            write!(out, "{} 0 obj\n{body}\nendobj\n", i + 1).unwrap();
        }
        let xref = out.len();
        write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
        for o in offsets {
            writeln!(out, "{o:010} 00000 n ").unwrap();
        }
        write!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .unwrap();
        out
    }

    /// A PDF `pdf-extract` 0.12 panics on (`unexpected encoding`): in the app's own process
    /// that panic is an abort, so this must only ever reach the child.
    fn bogus_encoding_pdf() -> Vec<u8> {
        pdf_with_encoding("Refunds are stored in cents", "Bogus")
    }

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    fn quick() -> Limits {
        Limits {
            time: Duration::from_secs(5),
            stdout: 1024,
            stderr: 64,
        }
    }

    #[test]
    fn the_reader_extracts_text() {
        let text = text_of(&pdf_with_encoding(
            "Refunds are stored in cents",
            "WinAnsiEncoding",
        ))
        .unwrap();
        assert!(text.contains("Refunds are stored in cents"), "{text:?}");
        assert!(text_of(b"not a pdf").is_err());
    }

    #[test]
    fn the_malformed_fixture_panics_pdf_extract_in_process() {
        // Why the reader runs in a child: this panic would abort the app in a release build.
        let fixture = bogus_encoding_pdf();
        let caught = std::panic::catch_unwind(|| text_of(&fixture));
        assert!(caught.is_err(), "pdf-extract no longer panics on it");
    }

    #[test]
    fn extracting_a_malformed_pdf_never_reads_it_in_process() {
        // Under `cargo test` the "reader" is the test harness, which refuses the flag: what
        // matters is that `extract` returns an error instead of panicking here.
        let err = crate::knowledge::extract("bad.pdf", &bogus_encoding_pdf()).unwrap_err();
        assert!(
            err.starts_with("bad.pdf isn't a PDF SayQL can read"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_childs_output_is_returned_and_its_input_given() {
        let out = run(sh("cat"), b"%PDF bytes".to_vec(), &quick()).unwrap();
        assert_eq!(out, b"%PDF bytes");
        let text = text_from(sh("cat >/dev/null; printf 'Refunds'"), "a.pdf", b"x").unwrap();
        assert_eq!(text, "Refunds");
    }

    #[cfg(unix)]
    #[test]
    fn a_crashing_reader_fails_only_that_file() {
        let err = run(sh("cat >/dev/null; kill -ABRT $$"), b"x".to_vec(), &quick()).unwrap_err();
        assert!(matches!(err, Failure::Exit { code: None, .. }), "{err:?}");
        let err = text_from(
            sh("echo 'thread main panicked' >&2; exit 101"),
            "a.pdf",
            b"x",
        )
        .unwrap_err();
        assert_eq!(err, "a.pdf isn't a PDF SayQL can read.");
        let err = text_from(sh("echo 'bad xref' >&2; exit 1"), "a.pdf", b"x").unwrap_err();
        assert_eq!(err, "a.pdf isn't a PDF SayQL can read (bad xref).");
    }

    #[cfg(unix)]
    #[test]
    fn a_reader_that_runs_too_long_is_killed() {
        let limits = Limits {
            time: Duration::from_millis(300),
            ..quick()
        };
        let started = Instant::now();
        let err = run(sh("sleep 30"), b"x".to_vec(), &limits).unwrap_err();
        assert!(matches!(err, Failure::TimedOut), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn a_reader_that_writes_too_much_is_killed() {
        let started = Instant::now();
        let err = run(sh("yes"), Vec::new(), &quick()).unwrap_err();
        assert!(matches!(err, Failure::TooMuchOutput), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(4));
        // Just over the cap, and exiting before it can be killed, still counts.
        let err = run(sh("head -c 1025 /dev/zero"), Vec::new(), &quick()).unwrap_err();
        assert!(matches!(err, Failure::TooMuchOutput), "{err:?}");
        assert_eq!(
            run(sh("head -c 1024 /dev/zero"), Vec::new(), &quick())
                .unwrap()
                .len(),
            1024
        );
    }

    #[cfg(unix)]
    #[test]
    fn stderr_is_capped_and_a_large_input_never_blocks() {
        // The child ignores stdin and floods stderr: neither pipe may deadlock the parent.
        let script = "head -c 1000000 /dev/zero >&2; exit 1";
        let err = run(sh(script), vec![0; 4 * 1024 * 1024], &quick()).unwrap_err();
        match err {
            Failure::Exit { code, stderr } => {
                assert_eq!(code, Some(1));
                assert_eq!(stderr.len(), 64);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_reader_is_an_error() {
        let err = run(Command::new("/no/such/sayql reader"), Vec::new(), &quick()).unwrap_err();
        assert!(matches!(err, Failure::Start(_)), "{err:?}");
    }

    /// The whole path against a built app: `SAYQL_BIN=target/release/sayql cargo test --lib
    /// real_reader -- --ignored`. A release build aborts on the malformed PDF.
    #[test]
    #[ignore = "needs a built sayql binary in SAYQL_BIN"]
    fn the_real_reader_reads_good_pdfs_and_survives_bad_ones() {
        let bin = std::env::var("SAYQL_BIN").unwrap();
        let reader = || {
            let mut cmd = Command::new(&bin);
            cmd.arg(FLAG);
            cmd
        };
        let good = pdf_with_encoding("Refunds are stored in cents", "WinAnsiEncoding");
        let text = text_from(reader(), "good.pdf", &good).unwrap();
        assert!(text.contains("Refunds are stored in cents"), "{text:?}");
        let err = text_from(reader(), "bad.pdf", &bogus_encoding_pdf()).unwrap_err();
        assert_eq!(err, "bad.pdf isn't a PDF SayQL can read.");
        let err = text_from(reader(), "x.pdf", b"not a pdf").unwrap_err();
        assert!(
            err.starts_with("x.pdf isn't a PDF SayQL can read (PDF error"),
            "{err}"
        );
    }
}
