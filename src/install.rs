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
use crate::registry;
use crate::resolve::{self, GithubCatalog, ResolvedPackage, DEFAULT_MAX_ITERATIONS};
use crate::source::{CustomSource, GithubSource};

/// The lockfile's file name within a project's subprojects directory. See
/// `crate::lockfile` for the format written there.
pub const LOCK_FILE_NAME: &str = "meson-jll.lock";

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
pub fn install(
    name: &str,
    version: Option<&str>,
    custom_url: Option<&str>,
    subprojects_dir: &Path,
    force: bool,
) -> Result<Vec<(String, String)>> {
    let bare_name = name.strip_suffix("_jll").unwrap_or(name).to_string();
    let lock_path = subprojects_dir.join(LOCK_FILE_NAME);
    let mut lock = LockFile::read(&lock_path)?;

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

        let mut resolved = resolve::resolve(&required, &pins, &catalog, DEFAULT_MAX_ITERATIONS)?;
        resolved.insert(
            bare_name.clone(),
            ResolvedPackage {
                name: bare_name.clone(),
                version: custom_package.version.clone(),
                dependencies: custom_package.dependencies.clone(),
            },
        );

        if previously_locked.get(&bare_name) != Some(&custom_package.version) {
            generate::write_wrap_set(&custom_package, subprojects_dir, force)?;
            installed.push((custom_package.name.clone(), custom_package.version.clone()));
        }
        already_generated.insert(bare_name.clone());

        resolved
    } else {
        let required: Vec<String> = lock.roots.keys().cloned().collect();
        resolve::resolve(&required, &pins, &catalog, DEFAULT_MAX_ITERATIONS)?
    };

    installed.extend(write_wraps_and_lock(
        &resolved,
        lock.roots,
        &lock_path,
        subprojects_dir,
        force,
        &already_generated,
        &previously_locked,
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
) -> Result<Vec<(String, String)>> {
    match name {
        Some(name) => install(name, None, None, subprojects_dir, force),
        None => update_all(subprojects_dir, force),
    }
}

fn update_all(subprojects_dir: &Path, force: bool) -> Result<Vec<(String, String)>> {
    let lock_path = subprojects_dir.join(LOCK_FILE_NAME);
    let mut lock = LockFile::read(&lock_path)?;
    if lock.roots.is_empty() {
        return Ok(Vec::new());
    }

    let previously_locked: HashMap<String, String> = lock
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();

    let required: Vec<String> = lock.roots.keys().cloned().collect();
    let catalog = GithubCatalog;
    let resolved = resolve::resolve(&required, &HashMap::new(), &catalog, DEFAULT_MAX_ITERATIONS)?;

    // Every root is moved to latest with no pins at all, so any version a
    // root was previously pinned to no longer applies. Reset every pin to
    // "*" rather than leave a stale version behind.
    for pin in lock.roots.values_mut() {
        *pin = "*".to_string();
    }

    write_wraps_and_lock(
        &resolved,
        lock.roots,
        &lock_path,
        subprojects_dir,
        force,
        &HashSet::new(),
        &previously_locked,
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
pub fn install_locked(subprojects_dir: &Path, force: bool) -> Result<Vec<(String, String)>> {
    let lock_path = subprojects_dir.join(LOCK_FILE_NAME);
    let lock = LockFile::read(&lock_path)?;

    let mut packages: Vec<&LockedPackage> = lock.packages.iter().collect();
    packages.sort_by(|left, right| left.name.cmp(&right.name));

    let mut installed = Vec::new();
    for package in packages {
        let full_package = load_at_locked_version(&package.name, &package.version)?;
        generate::write_wrap_set(&full_package, subprojects_dir, force)?;
        installed.push((full_package.name, full_package.version));
    }
    Ok(installed)
}

/// Regenerates every resolved package's wrap set and writes the lockfile.
///
/// `already_generated` skips names whose wrap set a caller already wrote
/// (used for a `--url` root, generated straight from its custom source
/// before this runs, so it is not fetched a second time through the
/// registry). `previously_locked` skips names whose resolved version is
/// unchanged from what was already locked: resolving always considers the
/// whole graph, so without this, every install or update would try to
/// rewrite every already-installed package's files too, which would fail
/// outright on an unrelated, unchanged package's wrap already existing
/// unless `--force` were given. The lockfile is written only after every
/// wrap set that needed writing has been generated successfully, so a
/// failure partway through never leaves the lock claiming a version whose
/// wrap was never actually written.
fn write_wraps_and_lock(
    resolved: &HashMap<String, ResolvedPackage>,
    roots: std::collections::BTreeMap<String, String>,
    lock_path: &Path,
    subprojects_dir: &Path,
    force: bool,
    already_generated: &HashSet<String>,
    previously_locked: &HashMap<String, String>,
) -> Result<Vec<(String, String)>> {
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
    LockFile {
        roots,
        packages: locked_packages,
    }
    .write(lock_path)?;

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
