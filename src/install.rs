//! Installing a JLL's wrap set together with its JLL dependencies.
//!
//! [`install_recursive`] is the shared core behind the `install` and
//! `update` subcommands. Both need to walk a JLL's dependency graph and
//! write a wrap set for every package in it, so this lives in one place
//! rather than being duplicated between the two commands.

use std::collections::HashSet;
use std::path::Path;

use crate::error::Result;
use crate::generate;
use crate::jll;
use crate::registry;
use crate::source::{CustomSource, GithubSource};

/// Installs `name`'s wrap set into `subprojects_dir`, and recursively every
/// JLL package it depends on.
///
/// `version` pins the top-level package to a specific JLL release (see
/// `info` for the versions available); when it is `None`, the latest commit
/// on `main` is used instead, and the version reported back is whatever
/// `Project.toml` declares there. `custom_url` overrides where the
/// top-level package's metadata is read from; its JLL dependencies are
/// always resolved through the registry, since they are ordinary published
/// JLLs rather than part of the custom source.
///
/// `visited` collects the bare names already written in this run, so a
/// dependency shared between two packages is generated only once even
/// across nested recursive calls. Returns the `(name, version)` of every
/// package written, in the order they were visited.
pub fn install_recursive(
    name: &str,
    version: Option<&str>,
    custom_url: Option<&str>,
    subprojects_dir: &Path,
    force: bool,
    visited: &mut HashSet<String>,
) -> Result<Vec<(String, String)>> {
    let bare_name = name.strip_suffix("_jll").unwrap_or(name).to_string();
    if visited.contains(&bare_name) {
        return Ok(Vec::new());
    }
    visited.insert(bare_name.clone());

    let git_ref = version
        .map(|version| format!("{bare_name}-v{version}"))
        .unwrap_or_else(|| "main".to_string());

    let package = if let Some(url) = custom_url {
        let source = CustomSource::parse(url, &git_ref);
        jll::load(&source)?
    } else {
        let (owner, repo) = registry::resolve(&bare_name);
        let source = GithubSource::new(owner, repo, git_ref);
        jll::load(&source)?
    };

    generate::write_wrap_set(&package, subprojects_dir, force)?;

    let mut installed = vec![(package.name.clone(), package.version.clone())];
    for dependency in &package.dependencies {
        installed.extend(install_recursive(
            dependency,
            None,
            None,
            subprojects_dir,
            force,
            visited,
        )?);
    }

    Ok(installed)
}
