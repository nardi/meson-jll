//! Installing and updating JLL wrap sets through the version resolver.
//!
//! This is the command layer described in `crate::internals`, "Resolving
//! versions": [`crate::resolve::resolve`] itself is stateless, it just
//! turns `required` names and `pins` into one resolved version per package.
//! What makes installing or updating one package leave every unrelated
//! package exactly where it was locked is entirely here, in how this module
//! builds those `pins` from the existing lockfile before calling it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::Result;
use crate::generate;
use crate::jll;
use crate::lock::{LockFile, LockedPackage};
use crate::progress::Progress;
use crate::registry;
use crate::resolve::{self, GithubCatalog, ResolvedPackage, DEFAULT_MAX_ITERATIONS};
use crate::source::{CustomSource, GithubSource};

/// The lockfile's file name. It is a wrap file inside the subprojects
/// directory, because Meson reads that file before `meson-jll` has run and
/// ignores the lock sections inside it. See `crate::lockfile` for the format.
pub const LOCK_FILE_NAME: &str = "meson-jll.lock.wrap";

/// The lockfile's path, given a project's subprojects directory. This is
/// the one place the "lockfile lives in the subprojects directory" rule is
/// encoded, so every command reads and writes the same location.
pub fn lock_path_for(subprojects_dir: &Path) -> std::path::PathBuf {
    subprojects_dir.join(LOCK_FILE_NAME)
}

/// The file name of the lockfile that earlier versions wrote, in the project
/// root. See [`read_lock`].
pub const LEGACY_LOCK_FILE_NAME: &str = "meson-jll.lock";

/// The path of the lockfile that earlier versions wrote, given a project's
/// subprojects directory. A bare relative `subprojects` has a parent of `""`
/// (the empty path), which is the current directory.
fn legacy_lock_path_for(subprojects_dir: &Path) -> std::path::PathBuf {
    let project_root = match subprojects_dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    project_root.join(LEGACY_LOCK_FILE_NAME)
}

/// Reads the lock of the project whose subprojects directory is
/// `subprojects_dir`.
///
/// Earlier versions kept the lock in a TOML file of its own, `meson-jll.lock`
/// in the project root. That file is read when the wrap file holds no lock,
/// so a project does not lose its pinned versions by upgrading. It is
/// replaced the next time the lock is written.
pub fn read_lock(subprojects_dir: &Path) -> Result<LockFile> {
    let lock = LockFile::read(&lock_path_for(subprojects_dir))?;
    if !lock.is_empty() {
        return Ok(lock);
    }
    LockFile::read_legacy(&legacy_lock_path_for(subprojects_dir))
}

/// Writes the lock into the wrap file of `subprojects_dir`, and removes the
/// lockfile of earlier versions if the project still has one, since the wrap
/// file holds the same information from now on.
///
/// A file that has no wrap section yet, as in a project that never had one,
/// starts with the sections `initial_wrap` returns, so the first write sets
/// up the fallback that fetches `meson-jll` itself. It is only called then.
fn write_lock(
    subprojects_dir: &Path,
    lock: &LockFile,
    initial_wrap: impl FnOnce() -> Option<String>,
) -> Result<()> {
    let path = lock_path_for(subprojects_dir);
    let wrap = if LockFile::has_wrap_section(&path)? {
        None
    } else {
        initial_wrap()
    };
    lock.write_starting_with(&path, wrap.as_deref())?;
    let legacy_path = legacy_lock_path_for(subprojects_dir);
    match std::fs::remove_file(&legacy_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::error::Error::WriteFile {
            path: legacy_path,
            source,
        }),
    }
}

/// A JLL's Windows binaries are MinGW-w64 built, and depend at runtime on
/// MinGW's own runtime DLLs (`libwinpthread-1.dll`, `libgcc_s_seh-1.dll`,
/// and, for Fortran code, `libgfortran`), which this package provides.
/// Nothing declares this as a real dependency anywhere: it is not in any
/// JLL's `Project.toml` `[deps]` (confirmed against SuiteSparse_jll's,
/// which lists none), since it is a property of the toolchain every
/// Windows JLL happens to be built with, not of the package itself. Every
/// resolve includes it unconditionally so `dependency('CompilerSupportLibraries_jll')`
/// always has a wrap to find, regardless of whether the specific JLLs
/// being installed turn out to need it, the same way every platform's
/// wraps are always generated even though only one is ever downloaded.
pub(crate) const WINDOWS_RUNTIME_SHIM_PACKAGE: &str = "CompilerSupportLibraries";

/// Installs or refreshes `name` in `subprojects_dir`, and every JLL package
/// it depends on.
///
/// `version` pins `name` to a specific JLL release, or refreshes it to the
/// latest available version when `None`. `custom_url`, when given, reads
/// `name`'s own metadata from that git URL or local path instead of the
/// `JuliaBinaryWrappers` organization, exactly like the existing `--url`
/// option; only `name`'s JLL dependencies are resolved through the normal
/// registry-backed catalog.
///
/// Every other root already in the lock, and every package outside `name`'s
/// own dependency closure as it was locked before this call, is pinned to
/// its current locked version, so installing or updating one package can
/// never move an unrelated one. A package inside that closure is free to
/// rise if `name`'s new requirements need a higher version of it.
///
/// Returns the `(name, version)` of every package written, sorted by name.
///
/// `on_progress` is called with each step's progress as it happens (see
/// [`Progress`]), so a caller can report it. It is called synchronously and
/// carries no timing information of its own, since network-bound work
/// happens between calls, not during one.
pub fn install(
    name: &str,
    version: Option<&str>,
    custom_url: Option<&str>,
    subprojects_dir: &Path,
    force: bool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Vec<(String, String)>> {
    let bare_name = if custom_url.is_some() {
        // A custom URL is not necessarily hosted under `JuliaBinaryWrappers`,
        // so its authoritative name comes from its own fetched
        // `Project.toml` further down instead of a registry lookup here.
        name.strip_suffix("_jll").unwrap_or(name).to_string()
    } else {
        registry::canonical_bare_name(name)?
    };
    let mut lock = read_lock(subprojects_dir)?;

    // Every package's version before this call, used below to skip
    // regenerating a wrap set that has not actually changed. Without this,
    // every install would try to rewrite every already-installed package's
    // files too, since resolving always considers the whole graph, and
    // that would fail outright on an unrelated, unchanged package's wrap
    // already existing (unless --force is given, which should not be
    // needed just to install something new).
    let previously_locked: HashMap<String, String> = lock
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();

    // Everything outside the package being installed's own closure keeps
    // exactly the version it was already locked to. The closure is read
    // from the OLD lock, since that is the only dependency graph known
    // before resolving again.
    let refreshed_closure = lock.closure(&bare_name);
    let mut pins: HashMap<String, String> = HashMap::new();
    for package in &lock.packages {
        if !refreshed_closure.contains(&package.name) {
            pins.insert(package.name.clone(), package.version.clone());
        }
    }

    // Always overwrite the root's recorded pin, not just fill it in when
    // absent: an install or update with no version means "track latest",
    // which must reset an old pin back to "*" rather than leave it
    // displaying a version this call no longer enforces.
    lock.roots.insert(
        bare_name.clone(),
        version.map_or_else(|| "*".to_string(), str::to_string),
    );
    if let Some(pinned_version) = version {
        // The closure loop above only pins packages OUTSIDE the refreshed
        // closure, and the package being installed is always inside its
        // own closure, so its own explicit version pin has to be added
        // separately here or `resolve` would never see it.
        pins.insert(bare_name.clone(), pinned_version.to_string());
    }

    let catalog = GithubCatalog;
    let mut installed: Vec<(String, String)> = Vec::new();
    let mut already_generated: HashSet<String> = HashSet::new();

    let resolved: HashMap<String, ResolvedPackage> = if let Some(url) = custom_url {
        // The custom root has no GitHub tag list to enumerate, so it is
        // loaded directly at a single fixed version, and only its own JLL
        // dependencies go through the registry-backed catalog.
        let git_ref = version
            .map(|pinned| format!("{bare_name}-v{pinned}"))
            .unwrap_or_else(|| "main".to_string());
        let source = CustomSource::parse(url, &git_ref);
        let custom_package = jll::load(&source)?;

        let mut required: Vec<String> = lock
            .roots
            .keys()
            .filter(|&root| *root != bare_name)
            .cloned()
            .collect();
        required.extend(custom_package.dependencies.iter().cloned());
        required.push(WINDOWS_RUNTIME_SHIM_PACKAGE.to_string());

        let mut resolved = resolve::resolve(
            &required,
            &pins,
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |resolving_name| on_progress(Progress::Resolving(resolving_name)),
        )?;
        resolved.insert(
            bare_name.clone(),
            ResolvedPackage {
                name: bare_name.clone(),
                version: custom_package.version.clone(),
                dependencies: custom_package.dependencies.clone(),
            },
        );
        on_progress(Progress::Resolved {
            count: resolved.len(),
        });

        if previously_locked.get(&bare_name) != Some(&custom_package.version) {
            generate::write_wrap_set(&custom_package, subprojects_dir, force)?;
            installed.push((custom_package.name.clone(), custom_package.version.clone()));
        }
        already_generated.insert(bare_name.clone());

        resolved
    } else {
        let mut required: Vec<String> = lock.roots.keys().cloned().collect();
        required.push(WINDOWS_RUNTIME_SHIM_PACKAGE.to_string());
        let resolved = resolve::resolve(
            &required,
            &pins,
            &catalog,
            DEFAULT_MAX_ITERATIONS,
            |resolving_name| on_progress(Progress::Resolving(resolving_name)),
        )?;
        on_progress(Progress::Resolved {
            count: resolved.len(),
        });
        resolved
    };

    installed.extend(write_wraps_and_lock(
        &resolved,
        lock.roots,
        WriteWrapsContext {
            subprojects_dir,
            force,
            already_generated: &already_generated,
            previously_locked: &previously_locked,
        },
        on_progress,
    )?);

    Ok(installed)
}

/// Refreshes `name` to its latest version, or every root at once when
/// `name` is `None`.
///
/// `update(Some(name), ...)` is exactly `install(name, None, None, ...)`:
/// installing with no version already means "take the latest", so updating
/// one package is not a separate code path. With no name, every root is
/// refreshed to latest at once, with no pins at all.
pub fn update(
    name: Option<&str>,
    subprojects_dir: &Path,
    force: bool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Vec<(String, String)>> {
    match name {
        Some(name) => install(name, None, None, subprojects_dir, force, on_progress),
        None => update_all(subprojects_dir, force, on_progress),
    }
}

fn update_all(
    subprojects_dir: &Path,
    force: bool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Vec<(String, String)>> {
    let mut lock = read_lock(subprojects_dir)?;
    if lock.roots.is_empty() {
        return Ok(Vec::new());
    }

    let previously_locked: HashMap<String, String> = lock
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();

    let mut required: Vec<String> = lock.roots.keys().cloned().collect();
    required.push(WINDOWS_RUNTIME_SHIM_PACKAGE.to_string());
    let catalog = GithubCatalog;
    let resolved = resolve::resolve(
        &required,
        &HashMap::new(),
        &catalog,
        DEFAULT_MAX_ITERATIONS,
        |resolving_name| on_progress(Progress::Resolving(resolving_name)),
    )?;
    on_progress(Progress::Resolved {
        count: resolved.len(),
    });

    // Every root is moved to latest with no pins at all, so any version a
    // root was previously pinned to no longer applies. Reset every pin to
    // "*" rather than leave a stale version behind.
    for pin in lock.roots.values_mut() {
        *pin = "*".to_string();
    }

    write_wraps_and_lock(
        &resolved,
        lock.roots,
        WriteWrapsContext {
            subprojects_dir,
            force,
            already_generated: &HashSet::new(),
            previously_locked: &previously_locked,
        },
        on_progress,
    )
}

/// Regenerates every wrap in `subprojects_dir` straight from its existing
/// lockfile, without resolving or contacting the registry for version
/// information. Deterministic and needs the network only to re-fetch each
/// locked package's own files.
///
/// A limitation worth knowing: the lock does not record whether a package
/// was originally installed from a custom `--url`, so a package installed
/// that way is re-fetched from the `JuliaBinaryWrappers` organization here
/// instead. Regenerating such a project from its lock alone is not yet
/// supported.
pub fn install_locked(
    subprojects_dir: &Path,
    force: bool,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Vec<(String, String)>> {
    let lock = read_lock(subprojects_dir)?;

    let mut packages: Vec<&LockedPackage> = lock.packages.iter().collect();
    packages.sort_by(|left, right| left.name.cmp(&right.name));

    let mut installed = Vec::new();
    for package in packages {
        on_progress(Progress::Writing(&package.name));
        let full_package = load_at_locked_version(&package.name, &package.version)?;
        generate::write_wrap_set(&full_package, subprojects_dir, force)?;
        installed.push((full_package.name, full_package.version));
    }
    Ok(installed)
}

/// The parts of [`write_wraps_and_lock`]'s job that stay the same across
/// its callers, grouped so the function itself does not need one parameter
/// per field.
struct WriteWrapsContext<'a> {
    subprojects_dir: &'a Path,
    force: bool,
    /// Skips names whose wrap set a caller already wrote (used for a
    /// `--url` root, generated straight from its custom source before this
    /// runs, so it is not fetched a second time through the registry).
    already_generated: &'a HashSet<String>,
    /// Skips names whose resolved version is unchanged from what was
    /// already locked: resolving always considers the whole graph, so
    /// without this, every install or update would try to rewrite every
    /// already-installed package's files too, which would fail outright on
    /// an unrelated, unchanged package's wrap already existing unless
    /// `--force` were given.
    previously_locked: &'a HashMap<String, String>,
}

/// Regenerates every resolved package's wrap set and writes the lockfile.
///
/// See [`WriteWrapsContext`] for what `already_generated` and
/// `previously_locked` skip. The lockfile is written only after every wrap
/// set that needed writing has been generated successfully, so a failure
/// partway through never leaves the lock claiming a version whose wrap was
/// never actually written.
fn write_wraps_and_lock(
    resolved: &HashMap<String, ResolvedPackage>,
    roots: std::collections::BTreeMap<String, String>,
    context: WriteWrapsContext,
    on_progress: &mut impl FnMut(Progress),
) -> Result<Vec<(String, String)>> {
    let WriteWrapsContext {
        subprojects_dir,
        force,
        already_generated,
        previously_locked,
    } = context;

    let mut installed = Vec::new();
    let mut package_names: Vec<&String> = resolved.keys().collect();
    package_names.sort();

    for package_name in package_names {
        if already_generated.contains(package_name) {
            continue;
        }
        let resolved_package = &resolved[package_name];
        if previously_locked.get(package_name) == Some(&resolved_package.version) {
            continue;
        }
        on_progress(Progress::Writing(package_name));
        let full_package = load_at_locked_version(package_name, &resolved_package.version)?;
        generate::write_wrap_set(&full_package, subprojects_dir, force)?;
        installed.push((full_package.name, full_package.version));
    }

    let locked_packages: Vec<LockedPackage> = resolved
        .values()
        .map(|package| LockedPackage {
            name: package.name.clone(),
            version: package.version.clone(),
            dependencies: package.dependencies.clone(),
        })
        .collect();
    write_lock(
        subprojects_dir,
        &LockFile {
            roots,
            packages: locked_packages,
        },
        crate::tool_wrap::running_release_wrap,
    )?;

    Ok(installed)
}

/// Loads a package's full metadata (platforms and library products
/// included, not just the dependency graph [`crate::resolve`] needs) at an
/// exact, already-resolved version, ready to generate its wrap set from.
fn load_at_locked_version(name: &str, version: &str) -> Result<jll::JllPackage> {
    let (owner, repo) = registry::resolve(name);
    let git_ref = format!("{name}-v{version}");
    let source = GithubSource::new(owner, repo, git_ref);
    jll::load(&source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const LEGACY_LOCK: &str = "version = 1\n\n[roots]\nExampleThing = \"*\"\n\n\
                               [[package]]\nname = \"ExampleThing\"\nversion = \"1.2.3+0\"\n\
                               dependencies = []\n";

    #[test]
    fn a_legacy_lock_is_read_when_the_wrap_has_none() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        fs::write(project.path().join("meson-jll.lock"), LEGACY_LOCK).unwrap();

        let lock = read_lock(&subprojects_dir).unwrap();
        assert_eq!(lock.packages.len(), 1);
        assert_eq!(lock.roots["ExampleThing"], "*");
    }

    #[test]
    fn the_lock_of_the_wrap_wins_over_a_legacy_lock() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        fs::write(project.path().join("meson-jll.lock"), LEGACY_LOCK).unwrap();
        LockFile {
            roots: [("OtherThing".to_string(), "*".to_string())].into(),
            packages: vec![],
        }
        .write(&lock_path_for(&subprojects_dir))
        .unwrap();

        let lock = read_lock(&subprojects_dir).unwrap();
        assert!(lock.roots.contains_key("OtherThing"));
        assert!(!lock.roots.contains_key("ExampleThing"));
    }

    #[test]
    fn the_first_write_starts_with_the_wrap_of_the_release() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        let wrap = "[wrap-file]\ndirectory = meson-jll-wrap-0.1.0\n";

        write_lock(&subprojects_dir, &LockFile::empty(), || {
            Some(wrap.to_string())
        })
        .unwrap();

        let text = fs::read_to_string(lock_path_for(&subprojects_dir)).unwrap();
        assert!(text.starts_with(wrap));
    }

    #[test]
    fn a_later_write_keeps_the_wrap_and_does_not_fetch_another() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        write_lock(&subprojects_dir, &LockFile::empty(), || {
            Some("[wrap-file]\ndirectory = first\n".to_string())
        })
        .unwrap();

        write_lock(&subprojects_dir, &LockFile::empty(), || {
            panic!("The wrap was fetched although the file already had one.")
        })
        .unwrap();

        let text = fs::read_to_string(lock_path_for(&subprojects_dir)).unwrap();
        assert!(text.starts_with("[wrap-file]\ndirectory = first"));
    }

    #[test]
    fn writing_the_lock_replaces_a_legacy_lock() {
        let project = tempfile::tempdir().unwrap();
        let subprojects_dir = project.path().join("subprojects");
        let legacy_path = project.path().join("meson-jll.lock");
        fs::write(&legacy_path, LEGACY_LOCK).unwrap();

        let lock = read_lock(&subprojects_dir).unwrap();
        write_lock(&subprojects_dir, &lock, || None).unwrap();

        assert!(!legacy_path.exists());
        let migrated = LockFile::read(&lock_path_for(&subprojects_dir)).unwrap();
        assert_eq!(migrated.packages, lock.packages);
        assert_eq!(read_lock(&subprojects_dir).unwrap().packages, lock.packages);
    }
}
