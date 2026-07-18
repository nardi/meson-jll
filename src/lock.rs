//! The committed lockfile that records which JLL versions are installed.
//!
//! The formal file format is specified in [`crate::lockfile`]. This module
//! is the one place that format is implemented: reading it (with a check
//! that the file's format `version` is one this code understands),
//! writing it back out in a stable order, and answering the one question
//! the command layer in [`crate::install`] needs from it, which packages
//! fall inside a given package's dependency closure.
//!
//! Keeping the lockfile as the single source of truth (rather than, say,
//! marker comments scattered across generated wrap files) is what lets
//! `meson-jll` tell an unrelated, already-installed package apart from the
//! one currently being installed or updated, without re-resolving anything
//! for it.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The lockfile format version this code reads and writes. Bumped only if
/// the shape of the file changes in a way older code could misread. See
/// [`crate::lockfile`] for the guarantee this gives a reader.
const FORMAT_VERSION: u32 = 1;

/// One package recorded in the lockfile: the version it resolved to, and
/// the bare names of its direct JLL dependencies at that version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// The on-disk shape of the lockfile. Kept separate from [`LockFile`] so
/// that the public type can stay a plain, always-valid value (for example,
/// its `packages` need not already be sorted), while this type mirrors the
/// TOML exactly for serde.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RawLockFile {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    roots: BTreeMap<String, String>,
    #[serde(default, rename = "package")]
    packages: Vec<LockedPackage>,
}

/// The resolved dependency graph of every JLL package installed in a
/// project, as read from or about to be written to `subprojects/*.lock`.
#[derive(Debug, Clone, Default)]
pub struct LockFile {
    /// The packages the user explicitly installed, mapped to their pin
    /// (`"*"` for unpinned, otherwise a concrete version). These are the
    /// entry points a re-resolve starts from.
    pub roots: BTreeMap<String, String>,
    /// Every package in the resolved graph, roots included.
    pub packages: Vec<LockedPackage>,
}

impl LockFile {
    /// An empty lock, as if nothing had ever been installed.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Reads the lockfile at `path`. A missing file is not an error, since
    /// that just means nothing has been installed yet, and reads back as
    /// [`Self::empty`]. A file that exists but declares a `version` this
    /// code does not understand is an error, so a future format change can
    /// never be silently misread.
    pub fn read(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(source) => {
                return Err(Error::ReadLocalFile {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        let raw: RawLockFile = toml::from_str(&text).map_err(|source| Error::ParseLockFile {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;

        match raw.version {
            Some(FORMAT_VERSION) => {}
            Some(found) => {
                return Err(Error::UnsupportedLockFileVersion {
                    path: path.to_path_buf(),
                    found,
                    supported: FORMAT_VERSION,
                });
            }
            None => {
                return Err(Error::UnsupportedLockFileVersion {
                    path: path.to_path_buf(),
                    found: 0,
                    supported: FORMAT_VERSION,
                });
            }
        }

        Ok(Self {
            roots: raw.roots,
            packages: raw.packages,
        })
    }

    /// Writes the lockfile to `path`. Packages are sorted by name first, so
    /// the file is stable across regenerations that resolve to the same
    /// versions, and diffs stay minimal.
    pub fn write(&self, path: &Path) -> Result<()> {
        let mut packages = self.packages.clone();
        packages.sort_by(|left, right| left.name.cmp(&right.name));

        let raw = RawLockFile {
            version: Some(FORMAT_VERSION),
            roots: self.roots.clone(),
            packages,
        };
        let text = toml::to_string_pretty(&raw).map_err(|source| Error::SerializeLockFile {
            source: Box::new(source),
        })?;
        fs::write(path, text).map_err(|source| Error::WriteFile {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Looks up a locked package by its bare name.
    pub fn get(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|package| package.name == name)
    }

    /// The transitive dependency closure of `name` within this lock,
    /// `name` itself included. A name outside the lock (nothing installed
    /// under that name yet) closes over just itself.
    ///
    /// This is how the command layer in [`crate::install`] decides which
    /// locked packages a refresh is allowed to move: everything in the
    /// closure of the package being installed or updated is free to move,
    /// and everything outside it is pinned to its current locked version.
    /// See `crate::internals`, "Resolving versions", for why that keeps
    /// unrelated roots exactly where they were locked.
    pub fn closure(&self, name: &str) -> HashSet<String> {
        let mut closure = HashSet::new();
        let mut pending = vec![name.to_string()];
        while let Some(current) = pending.pop() {
            if !closure.insert(current.clone()) {
                continue;
            }
            if let Some(package) = self.get(&current) {
                pending.extend(package.dependencies.iter().cloned());
            }
        }
        closure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LockFile {
        let mut roots = BTreeMap::new();
        roots.insert("ExampleThing".to_string(), "*".to_string());
        LockFile {
            roots,
            packages: vec![
                LockedPackage {
                    name: "ExampleThing".to_string(),
                    version: "1.2.3+0".to_string(),
                    dependencies: vec!["OtherThing".to_string()],
                },
                LockedPackage {
                    name: "OtherThing".to_string(),
                    version: "5.8.0+0".to_string(),
                    dependencies: vec![],
                },
            ],
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock");

        let lock = sample();
        lock.write(&path).unwrap();
        let read_back = LockFile::read(&path).unwrap();

        assert_eq!(read_back.roots, lock.roots);
        assert_eq!(read_back.packages, lock.packages);
    }

    #[test]
    fn a_missing_lockfile_reads_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("does-not-exist.lock");
        let lock = LockFile::read(&path).unwrap();
        assert!(lock.roots.is_empty());
        assert!(lock.packages.is_empty());
    }

    #[test]
    fn rejects_an_unrecognised_format_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock");
        fs::write(&path, "version = 99\n").unwrap();

        let error = LockFile::read(&path).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockFileVersion { found: 99, .. }
        ));
    }

    #[test]
    fn rejects_a_lockfile_missing_a_version_field() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock");
        fs::write(&path, "[roots]\n").unwrap();

        let error = LockFile::read(&path).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockFileVersion { found: 0, .. }
        ));
    }

    #[test]
    fn closure_includes_transitive_dependencies() {
        let lock = sample();
        let closure = lock.closure("ExampleThing");
        assert_eq!(
            closure,
            HashSet::from(["ExampleThing".to_string(), "OtherThing".to_string()])
        );
    }

    #[test]
    fn closure_of_an_unlocked_name_is_just_itself() {
        let lock = sample();
        let closure = lock.closure("NotInstalled");
        assert_eq!(closure, HashSet::from(["NotInstalled".to_string()]));
    }
}
