//! Discovering which JLL packages are already installed in a project.
//!
//! This reads [`crate::lock::LockFile`] directly. Earlier versions of this
//! tool had no lockfile and scanned each generated wrap file for a marker
//! comment instead; that marker is still written (see
//! [`crate::generate::context::SelectorWrapContext`]) as a human-readable
//! note, but is no longer what `status` or the install command layer read.

use std::path::Path;

use crate::error::Result;
use crate::install::LOCK_FILE_NAME;
use crate::lock::LockFile;

/// One JLL package already recorded in a project's lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
}

/// Reads every package recorded in `subprojects_dir`'s lockfile, sorted by
/// name. Returns an empty list if no lockfile exists yet.
pub fn installed_packages(subprojects_dir: &Path) -> Result<Vec<InstalledPackage>> {
    let lock_path = subprojects_dir.join(LOCK_FILE_NAME);
    let lock = LockFile::read(&lock_path)?;

    let mut packages: Vec<InstalledPackage> = lock
        .packages
        .into_iter()
        .map(|package| InstalledPackage {
            name: package.name,
            version: package.version,
        })
        .collect();
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockedPackage;

    #[test]
    fn reads_packages_from_the_lockfile_sorted_by_name() {
        let directory = tempfile::tempdir().unwrap();
        let lock_path = directory.path().join(LOCK_FILE_NAME);

        let lock = LockFile {
            roots: Default::default(),
            packages: vec![
                LockedPackage {
                    name: "OtherThing".to_string(),
                    version: "5.8.0+0".to_string(),
                    dependencies: vec![],
                },
                LockedPackage {
                    name: "ExampleThing".to_string(),
                    version: "1.2.3+0".to_string(),
                    dependencies: vec!["OtherThing".to_string()],
                },
            ],
        };
        lock.write(&lock_path).unwrap();

        let installed = installed_packages(directory.path()).unwrap();
        assert_eq!(
            installed,
            vec![
                InstalledPackage {
                    name: "ExampleThing".to_string(),
                    version: "1.2.3+0".to_string(),
                },
                InstalledPackage {
                    name: "OtherThing".to_string(),
                    version: "5.8.0+0".to_string(),
                },
            ]
        );
    }

    #[test]
    fn no_lockfile_reads_as_no_installed_packages() {
        let directory = tempfile::tempdir().unwrap();
        let installed = installed_packages(directory.path()).unwrap();
        assert!(installed.is_empty());
    }
}
