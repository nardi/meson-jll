//! The committed lockfile that records which JLL versions are installed.
//!
//! The lock lives inside a wrap file, `subprojects/meson-jll.lock.wrap`,
//! instead of a file of its own. Meson has to read a committed file before
//! `meson-jll` can run at all, so that file is the natural place to also
//! record what `meson-jll` resolved. Meson ignores any section of a wrap
//! file it does not know, which lets the lock share the file with the
//! wrap sections Meson does read. This module only ever touches the
//! sections named `meson-jll` or starting with `meson-jll.`, and keeps
//! every other line of the file as it found it.
//!
//! The formal file format is specified in [`crate::lockfile`]. This module
//! is the one place that format is implemented: reading it (with a check
//! that the file's `format` is one this code understands), writing it back
//! out in a stable order, and answering the one question the command layer
//! in [`crate::install`] needs from it, which packages fall inside a given
//! package's dependency closure.
//!
//! Keeping the lockfile as the single source of truth (rather than, say,
//! marker comments scattered across generated wrap files) is what lets
//! `meson-jll` tell an unrelated, already-installed package apart from the
//! one currently being installed or updated, without re-resolving anything
//! for it.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

/// The lockfile format version this code reads and writes. Bumped only if
/// the shape of the file changes in a way older code could misread. See
/// [`crate::lockfile`] for the guarantee this gives a reader.
const FORMAT_VERSION: u32 = 1;

/// The section that holds the format version.
const FORMAT_SECTION: &str = "meson-jll";

/// The section that maps each root package to its pin.
const ROOTS_SECTION: &str = "meson-jll.roots";

/// The prefix of the section that holds one locked package. The package's
/// bare name follows it.
const PACKAGE_SECTION_PREFIX: &str = "meson-jll.package.";

/// The text a new lockfile starts with. Meson rejects a wrap file that
/// does not begin with a wrap section, even when nothing refers to that
/// file, so a lockfile that does not share its file with a real wrap still
/// needs one.
const PLACEHOLDER_WRAP: &str = "\
# Meson rejects a wrap file that does not start with a wrap section.
# The sections named meson-jll below hold the lock.
[wrap-file]
directory = meson-jll
";

/// One package recorded in the lockfile: the version it resolved to, and
/// the bare names of its direct JLL dependencies at that version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub dependencies: Vec<String>,
}

/// The resolved dependency graph of every JLL package installed in a
/// project, as read from or about to be written to
/// `subprojects/meson-jll.lock.wrap`.
#[derive(Debug, Clone, Default)]
pub struct LockFile {
    /// The packages the user explicitly installed, mapped to their pin
    /// (`"*"` for unpinned, otherwise a concrete version). These are the
    /// entry points a re-resolve starts from.
    pub roots: BTreeMap<String, String>,
    /// Every package in the resolved graph, roots included.
    pub packages: Vec<LockedPackage>,
}

/// One `[section]` of an INI file together with the lines that follow its
/// header. The lines before the first header form a section without a name.
struct Section<'a> {
    /// The text between the brackets of the header, or `None` for the lines
    /// before the first header.
    name: Option<&'a str>,
    /// The number of the section's first line, counting from 1, used to
    /// point error messages at the right place.
    first_line: usize,
    /// Every line of the section exactly as written, header included.
    lines: Vec<&'a str>,
}

/// One `key = value` line of a section, with its surrounding whitespace
/// removed.
struct Entry<'a> {
    line: usize,
    key: &'a str,
    value: &'a str,
}

impl<'a> Section<'a> {
    /// Reads the `key = value` lines that follow the header, skipping blank
    /// lines and comments.
    fn entries(&self, path: &Path) -> Result<Vec<Entry<'a>>> {
        let mut entries = Vec::new();
        for (offset, &line) in self.lines.iter().enumerate().skip(1) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            let line_number = self.first_line + offset;
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(parse_error(
                    path,
                    line_number,
                    "Expected a line of the form `key = value`.",
                ));
            };
            entries.push(Entry {
                line: line_number,
                key: key.trim(),
                value: value.trim(),
            });
        }
        Ok(entries)
    }
}

/// Splits INI text into its sections, keeping every line so the file can be
/// written back without losing anything outside the lock.
fn split_sections(text: &str) -> Vec<Section<'_>> {
    let mut sections = vec![Section {
        name: None,
        first_line: 1,
        lines: Vec::new(),
    }];
    for (index, line) in text.lines().enumerate() {
        let header = line
            .trim()
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'));
        match header {
            Some(name) => sections.push(Section {
                name: Some(name),
                first_line: index + 1,
                lines: vec![line],
            }),
            None => sections
                .last_mut()
                .expect("The list always starts with the unnamed section.")
                .lines
                .push(line),
        }
    }
    sections
}

/// Whether a section belongs to the lock rather than to the wrap around it.
fn is_lock_section(name: &str) -> bool {
    name == FORMAT_SECTION || name.starts_with("meson-jll.")
}

fn parse_error(path: &Path, line: usize, reason: &str) -> Error {
    Error::ParseLockFile {
        path: path.to_path_buf(),
        line,
        reason: reason.to_string(),
    }
}

/// Reads one `meson-jll.package.<name>` section into a [`LockedPackage`].
fn read_package(section: &Section, name: &str, path: &Path) -> Result<LockedPackage> {
    let mut version = None;
    let mut dependencies = Vec::new();
    for entry in section.entries(path)? {
        match entry.key {
            "version" => version = Some(entry.value.to_string()),
            "dependencies" => {
                dependencies = entry
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|dependency| !dependency.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            _ => return Err(parse_error(path, entry.line, "Unknown key.")),
        }
    }
    let version = version.ok_or_else(|| {
        parse_error(
            path,
            section.first_line,
            "A package section needs a `version` key.",
        )
    })?;
    Ok(LockedPackage {
        name: name.to_string(),
        version,
        dependencies,
    })
}

/// Reads a file to a string, or `None` when it does not exist.
fn read_if_present(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::ReadLocalFile {
            path: path.to_path_buf(),
            source,
        }),
    }
}

impl LockFile {
    /// An empty lock, as if nothing had ever been installed.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Reads the lock from the wrap file at `path`. A missing file, or a
    /// file with no lock sections, is not an error, since that just means
    /// nothing has been installed yet, and reads back as [`Self::empty`].
    /// A file whose `format` this code does not understand is an error, so
    /// a future format change can never be silently misread.
    pub fn read(path: &Path) -> Result<Self> {
        let Some(text) = read_if_present(path)? else {
            return Ok(Self::empty());
        };
        let all_sections = split_sections(&text);
        let lock_sections: Vec<&Section> = all_sections
            .iter()
            .filter(|section| section.name.is_some_and(is_lock_section))
            .collect();
        if lock_sections.is_empty() {
            return Ok(Self::empty());
        }

        // The format is checked before anything else is read, so that a
        // newer format with sections this code has never heard of reports
        // the version problem instead of an unknown section.
        let mut format_version = None;
        for section in &lock_sections {
            if section.name == Some(FORMAT_SECTION) {
                for entry in section.entries(path)? {
                    if entry.key != "format" {
                        return Err(parse_error(path, entry.line, "Unknown key."));
                    }
                    let parsed = entry.value.parse::<u32>().map_err(|_| {
                        parse_error(path, entry.line, "The format must be a whole number.")
                    })?;
                    format_version = Some(parsed);
                }
            }
        }
        if format_version != Some(FORMAT_VERSION) {
            return Err(Error::UnsupportedLockFileVersion {
                path: path.to_path_buf(),
                found: format_version.unwrap_or(0),
                supported: FORMAT_VERSION,
            });
        }

        let mut roots = BTreeMap::new();
        let mut packages = Vec::new();
        for section in lock_sections {
            let name = section.name.expect("Lock sections always have a name.");
            if name == FORMAT_SECTION {
                continue;
            }
            if name == ROOTS_SECTION {
                for entry in section.entries(path)? {
                    roots.insert(entry.key.to_string(), entry.value.to_string());
                }
            } else if let Some(package_name) = name.strip_prefix(PACKAGE_SECTION_PREFIX) {
                packages.push(read_package(section, package_name, path)?);
            } else {
                return Err(parse_error(path, section.first_line, "Unknown section."));
            }
        }
        Ok(Self { roots, packages })
    }

    /// Writes the lock into the wrap file at `path`, replacing the lock
    /// sections it already holds and leaving every other line as it was.
    /// A file that does not exist yet starts with a placeholder wrap
    /// section. Packages are sorted by name first, so the file is stable
    /// across regenerations that resolve to the same versions, and diffs
    /// stay minimal.
    pub fn write(&self, path: &Path) -> Result<()> {
        let existing = read_if_present(path)?.unwrap_or_default();
        let all_sections = split_sections(&existing);
        let has_wrap_section = all_sections
            .iter()
            .any(|section| section.name.is_some_and(|name| !is_lock_section(name)));

        let mut text = String::new();
        if has_wrap_section {
            for section in &all_sections {
                if section.name.is_some_and(is_lock_section) {
                    continue;
                }
                for line in &section.lines {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        } else {
            text.push_str(PLACEHOLDER_WRAP);
        }
        let mut text = text.trim_end().to_string();
        text.push_str("\n\n");
        text.push_str(&self.lock_sections());

        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory).map_err(|source| Error::CreateDirectory {
                path: directory.to_path_buf(),
                source,
            })?;
        }
        fs::write(path, text).map_err(|source| Error::WriteFile {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Renders the lock sections, ending with a newline.
    fn lock_sections(&self) -> String {
        // The version of the tool that wrote the lock, so a bug report that
        // includes the file also names it.
        let mut text = format!(
            "[{FORMAT_SECTION}]\n# Written by meson-jll v{}\nformat = {FORMAT_VERSION}\n",
            env!("CARGO_PKG_VERSION")
        );
        if !self.roots.is_empty() {
            text.push_str(&format!("\n[{ROOTS_SECTION}]\n"));
            for (name, pin) in &self.roots {
                text.push_str(&format!("{name} = {pin}\n"));
            }
        }
        let mut packages: Vec<&LockedPackage> = self.packages.iter().collect();
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        for package in packages {
            text.push_str(&format!(
                "\n[{PACKAGE_SECTION_PREFIX}{}]\nversion = {}\n",
                package.name, package.version
            ));
            if !package.dependencies.is_empty() {
                text.push_str(&format!(
                    "dependencies = {}\n",
                    package.dependencies.join(", ")
                ));
            }
        }
        text
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
        let path = directory.path().join("meson-jll.lock.wrap");

        let lock = sample();
        lock.write(&path).unwrap();
        let read_back = LockFile::read(&path).unwrap();

        assert_eq!(read_back.roots, lock.roots);
        assert_eq!(read_back.packages, lock.packages);
    }

    #[test]
    fn a_new_lockfile_starts_with_a_wrap_section() {
        let directory = tempfile::tempdir().unwrap();
        // The subprojects directory does not exist yet.
        let path = directory.path().join("subprojects/meson-jll.lock.wrap");

        sample().write(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let first_section = text.lines().find(|line| line.starts_with('[')).unwrap();
        assert_eq!(first_section, "[wrap-file]");
    }

    #[test]
    fn writing_keeps_the_sections_around_the_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");
        let wrap =
            "[wrap-file]\ndirectory = meson-jll-0.1.0\n\n[provide]\nprogram_names = meson-jll\n";
        fs::write(&path, wrap).unwrap();

        sample().write(&path).unwrap();
        // A second write replaces the lock sections instead of adding to them.
        sample().write(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(wrap));
        assert_eq!(text.matches("[meson-jll]").count(), 1);
        assert_eq!(LockFile::read(&path).unwrap().packages, sample().packages);
    }

    #[test]
    fn writing_the_same_lock_twice_gives_identical_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");

        sample().write(&path).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        sample().write(&path).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn a_missing_lockfile_reads_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("does-not-exist.lock.wrap");
        let lock = LockFile::read(&path).unwrap();
        assert!(lock.roots.is_empty());
        assert!(lock.packages.is_empty());
    }

    #[test]
    fn a_wrap_without_lock_sections_reads_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");
        fs::write(&path, "[wrap-file]\ndirectory = meson-jll\n").unwrap();

        let lock = LockFile::read(&path).unwrap();
        assert!(lock.roots.is_empty());
        assert!(lock.packages.is_empty());
    }

    #[test]
    fn rejects_an_unrecognised_format_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");
        fs::write(&path, "[meson-jll]\nformat = 99\n\n[meson-jll.future]\n").unwrap();

        let error = LockFile::read(&path).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockFileVersion { found: 99, .. }
        ));
    }

    #[test]
    fn rejects_a_lockfile_missing_a_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");
        fs::write(&path, "[meson-jll.roots]\nExampleThing = *\n").unwrap();

        let error = LockFile::read(&path).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockFileVersion { found: 0, .. }
        ));
    }

    #[test]
    fn rejects_a_package_without_a_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meson-jll.lock.wrap");
        fs::write(
            &path,
            "[meson-jll]\nformat = 1\n\n[meson-jll.package.ExampleThing]\n",
        )
        .unwrap();

        let error = LockFile::read(&path).unwrap_err();
        assert!(matches!(error, Error::ParseLockFile { line: 4, .. }));
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
