//! Discovering which JLL wraps are already installed in a project.
//!
//! Every selector wrap this tool writes carries a one-line marker comment
//! recording the package name and the version it was generated from (see
//! [`crate::generate::context::SelectorWrapContext`]). That marker is all
//! `status` and `update` need to find what is already installed, without
//! re-fetching anything.

use std::fs;
use std::path::Path;

/// One JLL wrap already present in a project's `subprojects/` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

/// Scans `subprojects_dir` for selector wraps this tool generated, by
/// looking for the marker comment left in each one. Returns an empty list
/// if the directory does not exist yet.
pub fn installed_packages(subprojects_dir: &Path) -> Vec<InstalledPackage> {
    let Ok(entries) = fs::read_dir(subprojects_dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "wrap")
        })
        .filter_map(|entry| {
            let contents = fs::read_to_string(entry.path()).ok()?;
            parse_marker(contents.lines().next()?)
        })
        .collect()
}

/// Parses a marker line of the form
/// `# meson-jll: name=SuiteSparse version=7.12.1+0`.
fn parse_marker(line: &str) -> Option<InstalledPackage> {
    let rest = line.strip_prefix("# meson-jll: ")?;
    let mut name = None;
    let mut version = None;
    for field in rest.split_whitespace() {
        if let Some(value) = field.strip_prefix("name=") {
            name = Some(value.to_string());
        } else if let Some(value) = field.strip_prefix("version=") {
            version = Some(value.to_string());
        }
    }
    Some(InstalledPackage {
        name: name?,
        version: version?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_marker_line() {
        let package = parse_marker("# meson-jll: name=SuiteSparse version=7.12.1+0").unwrap();
        assert_eq!(package.name, "SuiteSparse");
        assert_eq!(package.version, "7.12.1+0");
    }

    #[test]
    fn rejects_an_unrelated_line() {
        assert_eq!(parse_marker("[wrap-file]"), None);
    }

    #[test]
    fn rejects_a_line_missing_a_field() {
        assert_eq!(parse_marker("# meson-jll: name=SuiteSparse"), None);
    }
}
