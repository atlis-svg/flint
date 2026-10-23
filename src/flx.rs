//! `.flx` — the published Flint movie format, and self-contained projectors.
//!
//! Layout: `FLX` magic, format version byte, u32 LE flags, then the movie
//! (a [`Document`] as JSON) compressed with DEFLATE.
//! A projector is the flint-player executable with an `.flx` appended, followed
//! by the payload length (u64 LE) and the trailer `FLXPROJ!`.

use crate::model::Document;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use std::io::{Read, Write};
use std::path::Path;

pub const MAGIC: &[u8; 3] = b"FLX";
pub const VERSION: u8 = 1;
const TRAILER: &[u8; 8] = b"FLXPROJ!";

pub fn encode(doc: &Document) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(json.len() / 4 + 8);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut enc = DeflateEncoder::new(out, Compression::best());
    enc.write_all(&json).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

pub fn decode(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() < 8 || &bytes[..3] != MAGIC {
        return Err("not a Flint movie (.flx)".into());
    }
    if bytes[3] > VERSION {
        return Err(format!("movie needs a newer player (format v{})", bytes[3]));
    }
    let mut json = Vec::new();
    DeflateDecoder::new(&bytes[8..]).read_to_end(&mut json).map_err(|e| format!("corrupt movie: {e}"))?;
    serde_json::from_slice(&json).map_err(|e| format!("corrupt movie: {e}"))
}

/// Loads a `.flx` movie or a `.flint` editor document.
pub fn load(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.starts_with(MAGIC) {
        decode(&bytes)
    } else {
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
}

pub fn save(doc: &Document, path: &Path) -> Result<usize, String> {
    let bytes = encode(doc)?;
    std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len())
}

/// Movie embedded in an executable (projector), if any.
pub fn embedded(exe: &[u8]) -> Option<&[u8]> {
    let n = exe.len();
    if n < 16 || &exe[n - 8..] != TRAILER {
        return None;
    }
    let len = u64::from_le_bytes(exe[n - 16..n - 8].try_into().ok()?) as usize;
    exe.get(n.checked_sub(16 + len)?..n - 16)
}

pub fn write_projector(player_exe: &Path, doc: &Document, out: &Path) -> Result<(), String> {
    let mut bytes = std::fs::read(player_exe).map_err(|e| format!("player not found at {}: {e}", player_exe.display()))?;
    if let Some(old) = embedded(&bytes) {
        // the player itself is a projector; strip its movie first
        let keep = bytes.len() - old.len() - 16;
        bytes.truncate(keep);
    }
    let movie = encode(doc)?;
    bytes.extend_from_slice(&movie);
    bytes.extend_from_slice(&(movie.len() as u64).to_le_bytes());
    bytes.extend_from_slice(TRAILER);
    std::fs::write(out, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Reads only the movie appended to the executable at `path`, if any.
pub fn embedded_in_file(path: &Path) -> Option<Result<Document, String>> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    if size < 16 {
        return None;
    }
    let mut tail = [0u8; 16];
    f.seek(SeekFrom::End(-16)).ok()?;
    f.read_exact(&mut tail).ok()?;
    if &tail[8..] != TRAILER {
        return None;
    }
    let len = u64::from_le_bytes(tail[..8].try_into().ok()?);
    if len + 16 > size {
        return None;
    }
    let mut movie = vec![0u8; len as usize];
    f.seek(SeekFrom::End(-16 - len as i64)).ok()?;
    f.read_exact(&mut movie).ok()?;
    Some(decode(&movie))
}

/// The flint-player executable that ships next to the running binary.
pub fn player_exe() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "flint-player.exe" } else { "flint-player" };
    let p = exe.with_file_name(name);
    p.exists().then_some(p)
}
