use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Error type for Tauri commands: any error becomes a readable string in the UI.
#[derive(Debug)]
pub struct Error(pub anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.into())
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{:#}", self.0))
    }
}

pub type Res<T> = Result<T, Error>;

/// Subprocess that never flashes a console window.
pub fn cmd(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    c.kill_on_drop(true);
    c
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Normalise ISO 639-1/2 codes and common names to a two-letter code.
pub fn lang2(code: &str) -> String {
    let c = code.trim().to_lowercase();
    let c = c.split(['-', '_']).next().unwrap_or("").to_string();
    match c.as_str() {
        "fre" | "fra" | "french" => "fr",
        "eng" | "english" => "en",
        "jpn" | "japanese" => "ja",
        "spa" | "spanish" => "es",
        "ger" | "deu" | "german" => "de",
        "ita" | "italian" => "it",
        "por" | "portuguese" => "pt",
        "kor" | "korean" => "ko",
        "chi" | "zho" | "chinese" => "zh",
        "rus" | "russian" => "ru",
        "dut" | "nld" | "dutch" => "nl",
        other => other,
    }
    .to_string()
}

/// Find a file by name anywhere under `dir` (archives nest their contents differently).
pub fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if p.file_name().and_then(|n| n.to_str()).map(|n| n.eq_ignore_ascii_case(name)) == Some(true) {
            return Some(p);
        }
    }
    subdirs.into_iter().find_map(|d| find_file(&d, name))
}

/// Locate an executable on PATH.
pub fn on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// Decode subtitle bytes: UTF-8 (with or without BOM), UTF-16, else Windows-1252.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode(&bytes[bom_len..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}
