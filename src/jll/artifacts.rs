//! Parsing a JLL's `Artifacts.toml`.
//!
//! Unlike `Project.toml`, this file has one top-level table per artifact,
//! named after the artifact rather than after a fixed key, so it is parsed
//! into a generic map first and then looked up by the package's bare name.
//! Each entry describes one platform: its selectors (architecture,
//! operating system, and so on) and the URL and hash of its tarball.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::jll::triplet::{Arch, CallAbi, Libc, Os, Triplet};

/// One platform's tarball, ready to become a per-triplet binary wrap.
#[derive(Debug, Clone)]
pub struct Platform {
    pub triplet: Triplet,
    pub source_url: String,
    pub source_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactEntryRaw {
    arch: String,
    os: String,
    #[serde(default)]
    libc: Option<String>,
    #[serde(default)]
    call_abi: Option<String>,
    #[serde(default)]
    cxxstring_abi: Option<String>,
    #[serde(default)]
    libgfortran_version: Option<String>,
    #[serde(default)]
    download: Vec<DownloadRaw>,
}

#[derive(Debug, Clone, Deserialize)]
struct DownloadRaw {
    url: String,
    sha256: String,
}

/// Parses `Artifacts.toml` and returns the platform entries for `package_name`
/// (the package's bare name, matching the top-level table Julia generates
/// for its own artifact).
///
/// Entries whose `arch` or `os` this tool does not recognise are skipped
/// rather than treated as an error, since new platforms are added to
/// Yggdrasil more often than this tool's platform list is updated.
pub fn parse(text: &str, package_name: &str) -> Result<Vec<Platform>> {
    let document: HashMap<String, Vec<ArtifactEntryRaw>> =
        toml::from_str(text).map_err(|source| Error::ParseArtifactsToml { source })?;

    let entries = document.get(package_name).cloned().unwrap_or_default();
    if entries.is_empty() {
        return Err(Error::NoPlatforms {
            name: package_name.to_string(),
        });
    }

    let platforms: Vec<Platform> = entries
        .into_iter()
        .filter_map(|entry| {
            let arch = Arch::parse(&entry.arch)?;
            let os = Os::parse(&entry.os)?;
            let libc = entry.libc.as_deref().and_then(Libc::parse);
            let call_abi = entry.call_abi.as_deref().and_then(CallAbi::parse);
            let download = entry.download.into_iter().next()?;

            Some(Platform {
                triplet: Triplet {
                    arch,
                    os,
                    libc,
                    call_abi,
                    cxxstring_abi: entry.cxxstring_abi,
                    libgfortran_version: entry.libgfortran_version,
                },
                source_url: download.url,
                source_hash: download.sha256,
            })
        })
        .collect();

    Ok(deduplicate_by_identifier(platforms))
}

/// Keeps only the first platform seen for each triplet identifier.
///
/// Some JLLs (`libblastrampoline_jll` among them) list several
/// `Artifacts.toml` entries that only differ by `julia_version`, a
/// selector this tool does not track since the generated wraps are not
/// specific to a Julia install. Those entries would otherwise collapse to
/// the same generated file name and collide when writing it twice.
fn deduplicate_by_identifier(platforms: Vec<Platform>) -> Vec<Platform> {
    let mut seen = std::collections::HashSet::new();
    platforms
        .into_iter()
        .filter(|platform| seen.insert(platform.triplet.identifier()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        [[SuiteSparse]]
        arch = "x86_64"
        os = "linux"
        libc = "glibc"
        git-tree-sha1 = "4041f7188b7c0cc6f93d0e24465b4ee01145e1d2"

            [[SuiteSparse.download]]
            url = "https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl/releases/download/SuiteSparse-v7.12.1+0/SuiteSparse.v7.12.1.x86_64-linux-gnu.tar.gz"
            sha256 = "7891c44ad3f5531f3198de6aa490130ed1c4a15fa45cd28f20201f5860979c93"

        [[SuiteSparse]]
        arch = "aarch64"
        os = "macos"

            [[SuiteSparse.download]]
            url = "https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl/releases/download/SuiteSparse-v7.12.1+0/SuiteSparse.v7.12.1.aarch64-apple-darwin.tar.gz"
            sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    "#;

    #[test]
    fn parses_two_platforms() {
        let platforms = parse(EXAMPLE, "SuiteSparse").unwrap();
        assert_eq!(platforms.len(), 2);
        assert_eq!(platforms[0].triplet.identifier(), "x86_64-linux-gnu");
        assert_eq!(platforms[1].triplet.identifier(), "aarch64-darwin");
    }

    #[test]
    fn missing_package_name_is_an_error() {
        let result = parse(EXAMPLE, "DoesNotExist");
        assert!(result.is_err());
    }
}
