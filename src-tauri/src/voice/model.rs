//! The Whisper model file: where it lives, downloading it once, and checking it.
//!
//! The model is whisper.cpp's English `base` model, quantized to 8 bits: about as accurate
//! as the full 148 MB `base.en` at about half the size, and clearly better than `tiny.en`
//! with names like `order_items`. It comes from the official ggerganov/whisper.cpp repository
//! on Hugging Face and is only used once its SHA-256 matches the one pinned here.

use crate::error::{AppError, AppResult};
use crate::store::digest::{hex, sha256_file};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MODEL_FILE: &str = "ggml-base.en-q8_0.bin";
pub const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q8_0.bin";
pub const MODEL_SHA256: &str = "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e";
pub const MODEL_BYTES: u64 = 81_781_811;

/// `<app data>/voice/ggml-base.en-q8_0.bin`.
pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join("voice").join(MODEL_FILE)
}

/// Where a download in progress goes, so a half-written file is never taken for the model.
fn partial_path(path: &Path) -> PathBuf {
    path.with_extension("bin.part")
}

/// Whether the model is downloaded. A cheap size check; the hash is checked before use.
pub fn is_downloaded(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() == MODEL_BYTES)
        .unwrap_or(false)
}

/// Checks the file against `expected`; a mismatch deletes it, so the next use downloads again.
pub fn verify(path: &Path, expected: &str) -> AppResult<()> {
    if sha256_file(path)? == expected {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(AppError::User(
        "The voice model was damaged and has been removed. Download it again.".into(),
    ))
}

/// Removes the model and any unfinished download.
pub fn remove(path: &Path) -> AppResult<()> {
    for p in [path.to_path_buf(), partial_path(path)] {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

/// Downloads `url` to `path`, reporting `(received, total)` bytes as it goes. The file only
/// appears at `path` once its hash matches `sha256`. Setting `cancel` stops the download.
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    sha256: &str,
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> AppResult<()> {
    const FAILED: &str = "The download failed. Check your connection and try again.";
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = partial_path(path);
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| FAILED)?;
    let total = response.content_length().unwrap_or(MODEL_BYTES);
    let mut file = std::fs::File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut stream = response.bytes_stream();
    let result: AppResult<()> = async {
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::Relaxed) {
                return Err("Download canceled.".into());
            }
            let chunk = chunk.map_err(|_| FAILED)?;
            file.write_all(&chunk)?;
            hasher.update(&chunk);
            received += chunk.len() as u64;
            progress(received, total);
        }
        file.flush()?;
        if hex(&hasher.finalize_reset()) != sha256 {
            return Err(
                "The downloaded model didn't match its checksum, so it wasn't used. Try again."
                    .into(),
            );
        }
        Ok(())
    }
    .await;
    drop(file);
    match result {
        Ok(()) => Ok(std::fs::rename(&part, path)?),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sayql-voice-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn model_lives_in_the_voice_folder_of_app_data() {
        let path = model_path(Path::new("/data/com.sayql.app"));
        assert_eq!(
            path,
            Path::new("/data/com.sayql.app/voice/ggml-base.en-q8_0.bin")
        );
        assert_eq!(
            partial_path(&path),
            Path::new("/data/com.sayql.app/voice/ggml-base.en-q8_0.bin.part")
        );
    }

    #[test]
    fn pinned_download_is_the_official_https_file() {
        assert!(MODEL_URL.starts_with("https://huggingface.co/ggerganov/whisper.cpp/"));
        assert!(MODEL_URL.ends_with(MODEL_FILE));
        assert_eq!(MODEL_SHA256.len(), 64);
        assert!(
            MODEL_SHA256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn sha256_matches_known_digest() {
        let dir = temp_dir("sha");
        let file = dir.join("abc");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_keeps_a_good_file_and_deletes_a_bad_one() {
        let dir = temp_dir("verify");
        let file = dir.join(MODEL_FILE);
        std::fs::write(&file, b"abc").unwrap();
        verify(
            &file,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        assert!(file.exists());
        assert!(verify(&file, MODEL_SHA256).is_err());
        assert!(!file.exists());
    }

    /// The real download from Hugging Face (82 MB): `cargo test --lib -- --ignored downloads`
    #[tokio::test]
    #[ignore]
    async fn downloads_and_checks_the_real_model() {
        let dir = temp_dir("download");
        let path = model_path(&dir);
        let cancel = AtomicBool::new(false);
        let mut updates = 0;
        download(
            &reqwest::Client::new(),
            MODEL_URL,
            MODEL_SHA256,
            &path,
            &cancel,
            |received, total| {
                updates += 1;
                assert!(received <= total);
            },
        )
        .await
        .unwrap();
        assert!(is_downloaded(&path));
        assert!(!partial_path(&path).exists());
        assert!(updates > 10);
        verify(&path, MODEL_SHA256).unwrap();
        // A wrong checksum is refused and leaves nothing behind.
        remove(&path).unwrap();
        let err = download(
            &reqwest::Client::new(),
            MODEL_URL,
            &"0".repeat(64),
            &path,
            &cancel,
            |_, _| {},
        )
        .await;
        assert!(err.is_err());
        assert!(!path.exists() && !partial_path(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_full_size_file_counts_as_downloaded() {
        let dir = temp_dir("size");
        let file = model_path(&dir);
        assert!(!is_downloaded(&file));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"partial").unwrap();
        assert!(!is_downloaded(&file));
        remove(&file).unwrap();
        assert!(!file.exists());
        // Removing what isn't there is fine.
        remove(&file).unwrap();
    }
}
