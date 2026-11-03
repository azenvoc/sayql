//! SHA-256 of files SayQL downloads and checks against a pinned hash: the Whisper model
//! (voice/model.rs) and the built-in OpenCode (audit/opencode.rs).

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
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
