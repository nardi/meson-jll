//! The parsed JLL package model.
//!
//! [`load`] turns the three metadata files a JLL repository publishes
//! (`Project.toml`, `Artifacts.toml`, and the per-platform wrapper scripts)
//! into one [`JllPackage`], which is everything [`crate::generate`] needs to
//! write a wrap set.

pub mod artifacts;
pub mod project;
pub mod triplet;
pub mod wrappers;

use crate::error::Result;
use crate::source::Source;
use artifacts::Platform;
use project::ProjectToml;
use wrappers::LibraryProduct;

/// One fully resolved JLL package, ready to generate a wrap set from.
#[derive(Debug)]
pub struct JllPackage {
    /// The bare package name, for example `ExampleThing`.
    pub name: String,
    /// The JLL release version, for example `7.12.1+0`.
    pub version: String,
    /// The bare names of the other JLL packages this one depends on.
    pub dependencies: Vec<String>,
    /// Every platform this JLL publishes a tarball for, each carrying its
    /// own library products where a matching wrapper script was found.
    pub platforms: Vec<ResolvedPlatform>,
}

/// A platform's tarball together with the library products its wrapper
/// script declares.
#[derive(Debug)]
pub struct ResolvedPlatform {
    pub platform: Platform,
    /// Empty when no wrapper script could be found or parsed for this
    /// platform. The generated overlay still declares a dependency, just
    /// without any libraries wired into it, since a missing wrapper file
    /// most likely means this tool's triplet-to-file-name guess is wrong for
    /// an unusual platform, not that the platform has no libraries.
    pub library_products: Vec<LibraryProduct>,
}

/// Loads a JLL package's metadata from `source`, which must already be
/// pinned to the git ref (a tag such as `ExampleThing-v1.2.3+0`, or `main`
/// for the latest commit) the caller wants to read.
///
/// The package's version is read from `Project.toml` itself rather than
/// passed in, since that file is the authoritative source for it regardless
/// of which ref was fetched.
///
/// `source` is a generic parameter rather than a trait object, so the
/// concrete source (GitHub or a custom `--url`) is resolved once at the call
/// site instead of through a dynamic dispatch on every fetch.
pub fn load<S: Source>(source: &S) -> Result<JllPackage> {
    let project_text = source.fetch("Project.toml")?;
    let project = ProjectToml::parse(&project_text)?;

    let artifacts_text = source.fetch("Artifacts.toml")?;
    let platforms = artifacts::parse(&artifacts_text, project.bare_name())?;

    let platforms = platforms
        .into_iter()
        .map(|platform| {
            let wrapper_path = format!(
                "src/wrappers/{}.jl",
                platform.triplet.julia_wrapper_identifier()
            );
            let library_products = source
                .fetch(&wrapper_path)
                .map(|text| wrappers::parse_library_products(&text))
                .unwrap_or_default();
            ResolvedPlatform {
                platform,
                library_products,
            }
        })
        .collect();

    Ok(JllPackage {
        name: project.bare_name().to_string(),
        version: project.version.clone(),
        dependencies: project.jll_dependencies(),
        platforms,
    })
}
