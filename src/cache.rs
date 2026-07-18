//! A small on-disk cache for values it is always safe to recompute.
//!
//! Every cache in this crate follows the same shape: read a value keyed by
//! something cheap to check freshness against (a git commit SHA), and if
//! that key does not match what is on disk, or nothing is on disk yet, or
//! what is on disk fails to parse, just recompute it and write the fresh
//! result back. A cache miss or a corrupt cache file is never an error,
//! only ever a reason to do the (otherwise unavoidable) work again.

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The directory every cache in this crate writes under, inside the OS's
/// own cache directory (for example `~/.cache/meson-jll` on Linux). `None`
/// if the OS cache directory cannot be determined, in which case caching is
/// simply skipped everywhere.
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("meson-jll"))
}

/// Reads and deserializes `path` as JSON, returning `None` on any failure:
/// a missing file, corrupt content, or a shape that no longer matches `T`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Best-effort write of `value` as JSON to `path`, creating its parent
/// directory if needed. Any failure is silently ignored: a cache write only
/// ever speeds up a later call, and must never turn into a hard error.
pub fn write_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(text) = serde_json::to_string(value) {
        let _ = fs::write(path, text);
    }
}
