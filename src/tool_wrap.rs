//! The wrap sections that let Meson fetch `meson-jll` itself.
//!
//! Every release attaches a `meson-jll.lock.wrap` file with the wrap for
//! that release, which points at binaries built from it. A project keeps
//! these sections at the top of its own `subprojects/meson-jll.lock.wrap`,
//! above the lock (see [`crate::lockfile`]). This module reads them from a
//! release, to set them up on the first `install` and to move them to
//! another release with `self-update`. It is the only code that knows where
//! releases publish them.

use std::path::Path;

use crate::error::{Error, Result};
use crate::install::lock_path_for;
use crate::lock::LockFile;

/// The environment variable that names where the wrap sections are read
/// from, a URL or a path. The default is the `meson-jll.lock.wrap` file
/// attached to a GitHub release, which holds the hashes of that release's
/// binaries. Reading from somewhere else helps with a mirror, and tests.
pub const WRAP_SOURCE_VARIABLE: &str = "MESON_JLL_WRAP";

/// The name of the wrap file that every release attaches.
const RELEASE_WRAP_NAME: &str = "meson-jll.lock.wrap";

/// The prefix of the `directory` key in the wrap of a release, which ends in
/// the version of the release.
const DIRECTORY_PREFIX: &str = "directory = meson-jll-wrap-";

/// What `self-update` found and did.
#[derive(Debug, PartialEq, Eq)]
pub struct SelfUpdate {
    /// The `meson-jll` version the project pinned before, or `None` when the
    /// wrap file had no wrap sections of a release.
    pub previous_version: Option<String>,
    /// The `meson-jll` version the project pins now.
    pub version: String,
}

impl SelfUpdate {
    /// Whether the project already pinned this version, so that nothing was
    /// written.
    pub fn is_unchanged(&self) -> bool {
        self.previous_version.as_deref() == Some(self.version.as_str())
    }
}

/// The URL of the wrap file attached to a release. `version` names a release
/// such as `0.1.0` (a leading `v` is ignored), and `None` names the latest
/// one.
fn release_wrap_url(version: Option<&str>) -> String {
    let repository = env!("CARGO_PKG_REPOSITORY");
    match version {
        Some(version) => {
            let version = version.strip_prefix('v').unwrap_or(version);
            format!("{repository}/releases/download/v{version}/{RELEASE_WRAP_NAME}")
        }
        None => format!("{repository}/releases/latest/download/{RELEASE_WRAP_NAME}"),
    }
}

/// The version of the release that a wrap's sections point at, from the name
/// of the directory the wrap extracts to.
pub fn wrap_version(wrap: &str) -> Option<&str> {
    wrap.lines()
        .find_map(|line| line.trim().strip_prefix(DIRECTORY_PREFIX))
        .map(str::trim)
}

/// Reads the wrap sections of a release and checks that they are wrap
/// sections that provide `meson-jll`, without lock sections of their own,
/// since a file with those would write the lock twice.
fn read_wrap(version: Option<&str>) -> Result<String> {
    let source = std::env::var(WRAP_SOURCE_VARIABLE).unwrap_or_else(|_| release_wrap_url(version));
    let text = if source.starts_with("https://") || source.starts_with("http://") {
        crate::source::fetch_url(&source)?
    } else {
        std::fs::read_to_string(&source).map_err(|error| Error::ReadLocalFile {
            path: source.clone().into(),
            source: error,
        })?
    };
    let provides_program = text.contains("[wrap-file]") && text.contains("program_names");
    let has_lock_sections = text
        .lines()
        .any(|line| line.trim().starts_with("[meson-jll"));
    if !provides_program || has_lock_sections || wrap_version(&text).is_none() {
        return Err(Error::InvalidToolWrap { location: source });
    }
    Ok(text)
}

/// The wrap sections of the release that is running, for a project that has
/// none yet. This returns `None` when they cannot be read, for example for a
/// build that is not a release, or without a network. A project is not worse
/// off for that than before: it only has no fallback for a machine without
/// `meson-jll`, and can get one later with `self-update`.
pub(crate) fn running_release_wrap() -> Option<String> {
    read_wrap(Some(env!("CARGO_PKG_VERSION"))).ok()
}

/// Moves the project whose subprojects directory is `subprojects_dir` to
/// another release of `meson-jll`, by replacing the wrap sections of its
/// `meson-jll.lock.wrap` and keeping the lock. `version` names the release,
/// and `None` names the latest one.
///
/// A project without wrap sections of a release, including one without a
/// `meson-jll.lock.wrap` at all, gets them.
pub fn self_update(subprojects_dir: &Path, version: Option<&str>) -> Result<SelfUpdate> {
    apply_wrap(subprojects_dir, &read_wrap(version)?)
}

/// Puts the wrap sections `wrap` into the project's `meson-jll.lock.wrap`
/// unless it already pins that version. See [`self_update`].
fn apply_wrap(subprojects_dir: &Path, wrap: &str) -> Result<SelfUpdate> {
    let path = lock_path_for(subprojects_dir);
    let new_version = wrap_version(wrap)
        .expect("A wrap that was read has a version.")
        .to_string();
    let previous_version = LockFile::wrap_text(&path)?
        .as_deref()
        .and_then(wrap_version)
        .map(str::to_string);
    let update = SelfUpdate {
        previous_version,
        version: new_version,
    };
    if !update.is_unchanged() {
        LockFile::replace_wrap(&path, wrap)?;
    }
    Ok(update)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn wrap_of(version: &str) -> String {
        format!(
            "[wrap-file]\ndirectory = meson-jll-wrap-{version}\n\n[provide]\nprogram_names = meson-jll\n"
        )
    }

    #[test]
    fn reads_the_version_of_a_wrap() {
        assert_eq!(wrap_version(&wrap_of("0.1.0")), Some("0.1.0"));
        assert_eq!(wrap_version("[wrap-file]\ndirectory = meson-jll\n"), None);
    }

    #[test]
    fn a_release_is_named_with_or_without_a_leading_v() {
        assert_eq!(
            release_wrap_url(Some("0.2.0")),
            release_wrap_url(Some("v0.2.0"))
        );
        assert!(release_wrap_url(Some("0.2.0"))
            .ends_with("/releases/download/v0.2.0/meson-jll.lock.wrap"));
        assert!(release_wrap_url(None).ends_with("/releases/latest/download/meson-jll.lock.wrap"));
    }

    #[test]
    fn updating_replaces_the_wrap_and_keeps_the_lock() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        let path = lock_path_for(&subprojects_dir);
        let lock = LockFile {
            roots: [("ExampleThing".to_string(), "*".to_string())].into(),
            packages: vec![crate::lock::LockedPackage {
                name: "ExampleThing".to_string(),
                version: "1.2.3+0".to_string(),
                dependencies: vec![],
            }],
        };
        lock.write_starting_with(&path, Some(&wrap_of("0.1.0")))
            .unwrap();

        let update = apply_wrap(&subprojects_dir, &wrap_of("0.2.0")).unwrap();
        let again = apply_wrap(&subprojects_dir, &wrap_of("0.2.0")).unwrap();

        assert_eq!(update.previous_version.as_deref(), Some("0.1.0"));
        assert_eq!(update.version, "0.2.0");
        assert!(again.is_unchanged());
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(&wrap_of("0.2.0")));
        assert!(!text.contains("meson-jll-wrap-0.1.0"));
        assert_eq!(LockFile::read(&path).unwrap().packages, lock.packages);
    }

    #[test]
    fn a_project_without_a_wrap_file_gets_one() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");

        let update = apply_wrap(&subprojects_dir, &wrap_of("0.2.0")).unwrap();

        assert_eq!(update.previous_version, None);
        let text = fs::read_to_string(lock_path_for(&subprojects_dir)).unwrap();
        assert_eq!(text, wrap_of("0.2.0"));
    }
}
