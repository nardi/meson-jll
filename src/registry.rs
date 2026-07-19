//! Resolving a JLL package name to a repository, enumerating every JLL
//! package that exists, and listing a JLL's versions.
//!
//! Every JLL published through Yggdrasil ends up as a repository under the
//! `JuliaBinaryWrappers` GitHub organization, named `<Name>_jll.jl`.
//! Per-package lookups (a repository's tags, the highest of them) go
//! through git's own protocol (see `crate::git`) rather than the GitHub
//! REST API, since a JLL with several dependencies makes one such lookup
//! per dependency, and the API's 60-requests-per-hour unauthenticated rate
//! limit does not leave much room for that. Enumerating every JLL that
//! exists (for `list` and `search`) is answered from a different
//! repository instead: `JuliaPackaging/Yggdrasil`, the monorepo of build
//! recipes every JLL is built from. Its directory layout names every
//! buildable package directly, in one request (still through the REST API,
//! but one shared, cached request rather than one per package), which is
//! far cheaper than paginating the `JuliaBinaryWrappers` organization's
//! entire repository listing (roughly 700 repositories, split across many
//! pages) just to read off names.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache;
use crate::error::{Error, Result};
use crate::git;
use crate::source::fetch_url;
use crate::version::Version;

/// The GitHub organization every JLL package is published under.
const ORGANIZATION: &str = "JuliaBinaryWrappers";

/// The repository whose directory layout enumerates every buildable JLL.
const YGGDRASIL_OWNER: &str = "JuliaPackaging";
const YGGDRASIL_REPO: &str = "Yggdrasil";

/// Resolves a JLL name (bare, like `ExampleThing`, or full, like
/// `ExampleThing_jll`) to its `(owner, repo)` on GitHub.
pub fn resolve(name: &str) -> (String, String) {
    let bare = name.strip_suffix("_jll").unwrap_or(name);
    (ORGANIZATION.to_string(), format!("{bare}_jll.jl"))
}

/// Lists the release tags of a JLL repository, over git's own protocol
/// rather than the GitHub API (see the module documentation), each paired
/// with the commit it points at. Tag names look like `ExampleThing-v1.2.3+0`.
///
/// A repository that does not exist at all reads as an empty list, the
/// same outcome as a real JLL with zero releases, rather than a hard
/// error (see `crate::git::ls_remote_tags`), since a name reached only
/// transitively that turns out unpublished should not abort a whole
/// resolve (see [`crate::resolve::Catalog::versions`]).
pub fn list_tags(owner: &str, repo: &str) -> Result<Vec<(String, String)>> {
    let url = format!("https://github.com/{owner}/{repo}.git");
    git::ls_remote_tags(&url)
}

/// Extracts the JLL version (for example `1.2.3+0`) from a release tag
/// name (for example `ExampleThing-v1.2.3+0`).
pub fn version_from_tag(tag: &str) -> Option<&str> {
    tag.rsplit_once("-v").map(|(_, version)| version)
}

/// The most recently published version of a JLL package: the highest
/// version among its tags, compared the same way [`crate::resolve`]
/// compares every other version, rather than relying on tag order.
pub fn latest_version(owner: &str, repo: &str) -> Result<String> {
    let tags = list_tags(owner, repo)?;
    tags.iter()
        .filter_map(|(tag, _sha)| version_from_tag(tag))
        .filter_map(|raw| Version::parse(raw).ok().map(|version| (raw, version)))
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|(raw, _)| raw.to_string())
        .ok_or_else(|| Error::NoPlatforms {
            name: repo.to_string(),
        })
}

/// Looks up `name`'s exact, correctly cased bare name, and confirms it is
/// one of the packages Yggdrasil can build (see [`yggdrasil_package_names`]).
///
/// The comparison is case-insensitive, so this succeeds no matter which
/// case `name` was typed in, but the result is only returned as-is when it
/// matches the case actually requested. Otherwise this returns
/// [`Error::WrongCase`] with the real name as a suggestion, rather than
/// silently continuing under a different name than the one the caller
/// asked for: JLL names are case-sensitive everywhere else (a generated
/// `dependency()` name, a lockfile key, a raw content URL), so a mismatch
/// here is far more likely to be a typo than an intentional choice.
///
/// Returns [`Error::UnknownJllPackage`] if no such package exists at all,
/// case included.
pub fn canonical_bare_name(name: &str) -> Result<String> {
    let requested_bare = name.strip_suffix("_jll").unwrap_or(name);
    let requested_lower = requested_bare.to_lowercase();
    let names = yggdrasil_package_names()?;
    let actual_bare = names
        .into_iter()
        .find(|candidate| candidate.to_lowercase() == requested_lower)
        .ok_or_else(|| Error::UnknownJllPackage {
            name: requested_bare.to_string(),
        })?;
    if actual_bare == requested_bare {
        Ok(actual_bare)
    } else {
        Err(Error::WrongCase {
            given: requested_bare.to_string(),
            suggested: actual_bare,
        })
    }
}

/// Lists the bare names of every JLL package buildable in Yggdrasil.
///
/// See [`yggdrasil_package_names`] for how this is obtained and why it is
/// preferred over enumerating `JuliaBinaryWrappers` directly.
pub fn list_jll_packages() -> Result<Vec<String>> {
    yggdrasil_package_names()
}

/// Lists the bare names of every JLL package (per [`list_jll_packages`])
/// whose name contains `term`, case-insensitively.
pub fn search_jll_packages(term: &str) -> Result<Vec<String>> {
    let term = term.to_lowercase();
    let mut names = yggdrasil_package_names()?;
    names.retain(|name| name.to_lowercase().contains(&term));
    Ok(names)
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
struct RefResponse {
    object: RefObject,
}

#[derive(Debug, Deserialize)]
struct RefObject {
    sha: String,
}

/// One cached result of [`yggdrasil_package_names`], keyed by the
/// Yggdrasil commit it was computed from.
#[derive(Debug, Serialize, Deserialize)]
struct CachedPackageList {
    sha: String,
    names: Vec<String>,
}

/// Lists the bare names of every JLL package that Yggdrasil has a build
/// recipe for.
///
/// Yggdrasil (`JuliaPackaging/Yggdrasil`) is the monorepo every JLL is built
/// from. Each buildable package gets its own directory, directly under a
/// single-uppercase-letter bucket directory, for example
/// `Z/Zlib/build_tarballs.jl` for the `Zlib` package (a deeper directory
/// like `Z/Zlib/Zlib@1.2.12` is a per-version build recipe subdirectory, not
/// a separate package, and is excluded). Reading this one directory tree
/// answers "what JLL packages exist" in a single request, at a size (a few
/// thousand entries, a few megabytes) nowhere near GitHub's git-tree API
/// truncation limit, which is far cheaper than paginating the
/// `JuliaBinaryWrappers` organization's own repository listing.
///
/// A name found this way is a buildable recipe, not a guarantee that
/// `JuliaBinaryWrappers/<Name>_jll.jl` was ever successfully published (a
/// recipe can exist without having built successfully). [`canonical_bare_name`]
/// only checks against this same list, so it cannot catch that case either,
/// but [`crate::resolve::resolve`] still does: a required package with no
/// tags at all (see [`list_tags`]) fails with [`Error::UnknownJllPackage`]
/// there, just later in the pipeline than a case mismatch would.
///
/// The result is cached locally, keyed by Yggdrasil's current `master`
/// commit, so a repeat call in an unchanged Yggdrasil costs one small
/// request (checking the current commit) instead of refetching the whole
/// tree. See `read_cached_package_list` and `write_cached_package_list`.
pub fn yggdrasil_package_names() -> Result<Vec<String>> {
    let sha = fetch_yggdrasil_master_sha()?;
    let cache_path = cache_file_path();
    if let Some(path) = &cache_path {
        if let Some(names) = read_cached_package_list(&sha, path) {
            return Ok(names);
        }
    }

    let tree = fetch_yggdrasil_tree()?;
    let names = extract_package_names(&tree);

    if let Some(path) = &cache_path {
        write_cached_package_list(&sha, &names, path);
    }
    Ok(names)
}

/// The current commit at the tip of Yggdrasil's `master` branch, used as
/// the cache key for [`yggdrasil_package_names`].
fn fetch_yggdrasil_master_sha() -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{YGGDRASIL_OWNER}/{YGGDRASIL_REPO}/git/refs/heads/master"
    );
    let body = fetch_url(&url)?;
    let response: RefResponse =
        serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
    Ok(response.object.sha)
}

/// Fetches Yggdrasil's whole file tree in one request.
fn fetch_yggdrasil_tree() -> Result<Vec<TreeEntry>> {
    let url = format!(
        "https://api.github.com/repos/{YGGDRASIL_OWNER}/{YGGDRASIL_REPO}/git/trees/master?recursive=1"
    );
    let body = fetch_url(&url)?;
    let response: TreeResponse =
        serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
    Ok(response.tree)
}

/// `true` for a single uppercase ASCII letter, the bucket directories
/// Yggdrasil groups packages under (for example `Z` for `Zlib`).
fn is_bucket_letter(segment: &str) -> bool {
    segment.len() == 1
        && segment
            .chars()
            .next()
            .is_some_and(|letter| letter.is_ascii_uppercase())
}

/// Pulls the bare package names out of a Yggdrasil tree listing: every
/// directory exactly two path segments deep, whose first segment is a
/// bucket letter, sorted and deduplicated.
fn extract_package_names(entries: &[TreeEntry]) -> Vec<String> {
    let mut names: Vec<String> = entries
        .iter()
        .filter(|entry| entry.entry_type == "tree")
        .filter_map(
            |entry| match entry.path.split('/').collect::<Vec<_>>().as_slice() {
                [bucket, name] if is_bucket_letter(bucket) => Some((*name).to_string()),
                _ => None,
            },
        )
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Where [`yggdrasil_package_names`] caches its result, in the OS's own
/// cache directory, not inside any one project, since the result has
/// nothing to do with a particular project. `None` if the OS cache
/// directory cannot be determined, in which case caching is simply
/// skipped. Unlike the archive cache in [`crate::source`], this is a single
/// file rather than one per key, since there is only ever one Yggdrasil
/// package list worth having at a time, and a new one fully supersedes the
/// last.
fn cache_file_path() -> Option<PathBuf> {
    cache::cache_dir().map(|dir| dir.join("yggdrasil-packages.json"))
}

/// Reads a cached package list from `path`, returning it only if it was
/// computed from the same commit as `sha`.
fn read_cached_package_list(sha: &str, path: &Path) -> Option<Vec<String>> {
    let cached: CachedPackageList = cache::read_json(path)?;
    (cached.sha == sha).then_some(cached.names)
}

/// Writes a cached package list to `path`, keyed by the commit it was
/// computed from.
fn write_cached_package_list(sha: &str, names: &[String], path: &Path) {
    let cached = CachedPackageList {
        sha: sha.to_string(),
        names: names.to_vec(),
    };
    cache::write_json(path, &cached);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_bare_name() {
        assert_eq!(
            resolve("ExampleThing"),
            (
                "JuliaBinaryWrappers".to_string(),
                "ExampleThing_jll.jl".to_string()
            )
        );
    }

    #[test]
    fn resolves_a_full_name() {
        assert_eq!(
            resolve("ExampleThing_jll"),
            (
                "JuliaBinaryWrappers".to_string(),
                "ExampleThing_jll.jl".to_string()
            )
        );
    }

    #[test]
    fn extracts_version_from_tag() {
        assert_eq!(version_from_tag("ExampleThing-v1.2.3+0"), Some("1.2.3+0"));
        assert_eq!(version_from_tag("not-a-version-tag"), Some("ersion-tag"));
        assert_eq!(version_from_tag("notag"), None);
    }

    fn tree_entry(path: &str, entry_type: &str) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            entry_type: entry_type.to_string(),
        }
    }

    #[test]
    fn extracts_package_directories_directly_under_a_bucket_letter() {
        let entries = vec![
            tree_entry("Z/Zlib", "tree"),
            tree_entry("Z/ZlibNG", "tree"),
            tree_entry("A/ABC", "tree"),
        ];
        assert_eq!(
            extract_package_names(&entries),
            vec!["ABC".to_string(), "Zlib".to_string(), "ZlibNG".to_string()]
        );
    }

    #[test]
    fn excludes_per_version_build_recipe_subdirectories() {
        let entries = vec![
            tree_entry("Z/Zlib", "tree"),
            tree_entry("Z/Zlib/Zlib@1.2.12", "tree"),
            tree_entry("Z/Zlib/Zlib@1.2.12/build_tarballs.jl", "blob"),
            tree_entry("Z/Zlib/common.jl", "blob"),
        ];
        assert_eq!(extract_package_names(&entries), vec!["Zlib".to_string()]);
    }

    #[test]
    fn excludes_the_root_fs_bucket_and_non_tree_entries() {
        let entries = vec![
            tree_entry("0_RootFS/GCCBootstrap@11", "tree"),
            tree_entry("0_RootFS/common.jl", "blob"),
            tree_entry("Z/Zlib", "blob"),
        ];
        assert_eq!(extract_package_names(&entries), Vec::<String>::new());
    }

    #[test]
    fn cache_round_trips_on_a_matching_sha() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yggdrasil-packages.json");
        let names = vec!["Zlib".to_string(), "ZlibNG".to_string()];

        write_cached_package_list("abc123", &names, &path);

        assert_eq!(
            read_cached_package_list("abc123", &path),
            Some(names.clone())
        );
        assert_eq!(read_cached_package_list("different-sha", &path), None);
    }

    #[test]
    fn missing_cache_file_is_a_plain_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(read_cached_package_list("abc123", &path), None);
    }
}
