//! Resolving a JLL package name to a repository, enumerating every JLL
//! package that exists, and listing a JLL's versions, all through the
//! GitHub API.
//!
//! Every JLL published through Yggdrasil ends up as a repository under the
//! `JuliaBinaryWrappers` GitHub organization, named `<Name>_jll.jl`. That
//! makes a single git ref and repository lookup enough to answer "what
//! versions does this JLL have" and "what is this JLL's exact, correctly
//! cased name". Enumerating every JLL that exists (for `list` and `search`)
//! is answered from a different repository instead: `JuliaPackaging/Yggdrasil`,
//! the monorepo of build recipes every JLL is built from. Its directory
//! layout names every buildable package directly, in one request, which is
//! far cheaper than paginating the `JuliaBinaryWrappers` organization's
//! entire repository listing (roughly 700 repositories, split across many
//! pages) just to read off names.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::source::fetch_url;

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

#[derive(Debug, Deserialize)]
struct TagResponse {
    name: String,
}

/// Lists the release tags of a JLL repository, most recent first, as
/// reported by the GitHub API. Tag names look like `ExampleThing-v1.2.3+0`.
pub fn list_tags(owner: &str, repo: &str) -> Result<Vec<String>> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/tags?per_page=100");
    let body = fetch_url(&url)?;
    let tags: Vec<TagResponse> =
        serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
    Ok(tags.into_iter().map(|tag| tag.name).collect())
}

/// Extracts the JLL version (for example `1.2.3+0`) from a release tag
/// name (for example `ExampleThing-v1.2.3+0`).
pub fn version_from_tag(tag: &str) -> Option<&str> {
    tag.rsplit_once("-v").map(|(_, version)| version)
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

/// The most recently published version of a JLL package.
///
/// Reads GitHub's own "latest release" endpoint, which the API defines as
/// the most recently created non-prerelease, non-draft release, rather than
/// listing every tag and taking the first: one small request for one
/// release instead of a full (up to 100-tag) page just to read off its
/// first entry.
pub fn latest_version(owner: &str, repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let body = fetch_url(&url).map_err(|error| match error {
        Error::Fetch { source, .. } if matches!(source.as_ref(), ureq::Error::Status(404, _)) => {
            Error::NoPlatforms {
                name: repo.to_string(),
            }
        }
        other => other,
    })?;
    let response: ReleaseResponse =
        serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
    version_from_tag(&response.tag_name)
        .map(String::from)
        .ok_or_else(|| Error::NoPlatforms {
            name: repo.to_string(),
        })
}

#[derive(Debug, Deserialize)]
struct RepoResponse {
    name: String,
}

/// Looks up `name`'s exact, correctly cased bare name from the
/// `JuliaBinaryWrappers` organization, and confirms a repository actually
/// exists for it.
///
/// GitHub's REST API matches a repository path case-insensitively, so this
/// succeeds no matter which case `name` was typed in, but the result is
/// only returned as-is when it matches the case actually requested.
/// Otherwise this returns [`Error::WrongCase`] with the real name as a
/// suggestion, rather than silently continuing under a different name than
/// the one the caller asked for: JLL names are case-sensitive everywhere
/// else (a generated `dependency()` name, a lockfile key, a raw content
/// URL), so a mismatch here is far more likely to be a typo than an
/// intentional choice.
///
/// Returns [`Error::UnknownJllPackage`] if no such repository exists at
/// all, case included.
pub fn canonical_bare_name(name: &str) -> Result<String> {
    let requested_bare = name.strip_suffix("_jll").unwrap_or(name);
    let (owner, repo) = resolve(name);
    let url = format!("https://api.github.com/repos/{owner}/{repo}");
    let body = fetch_url(&url).map_err(|error| match error {
        Error::Fetch { source, .. } if matches!(source.as_ref(), ureq::Error::Status(404, _)) => {
            Error::UnknownJllPackage {
                name: requested_bare.to_string(),
            }
        }
        other => other,
    })?;
    let response: RepoResponse =
        serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
    let actual_bare = response
        .name
        .strip_suffix("_jll.jl")
        .unwrap_or(&response.name);
    if actual_bare == requested_bare {
        Ok(actual_bare.to_string())
    } else {
        Err(Error::WrongCase {
            given: requested_bare.to_string(),
            suggested: actual_bare.to_string(),
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
/// recipe can exist without having built successfully). [`crate::install`]
/// independently confirms the real repository exists through
/// [`canonical_bare_name`], and reports [`Error::UnknownJllPackage`] clearly
/// if a name from here turns out not to have a published repository.
///
/// The result is cached locally, keyed by Yggdrasil's current `master`
/// commit, so a repeat call in an unchanged Yggdrasil costs one small
/// request (checking the current commit) instead of refetching the whole
/// tree. See [`read_cached_package_list`] and [`write_cached_package_list`].
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
/// cache directory (for example `~/.cache/meson-jll` on Linux), not inside
/// any one project, since the result has nothing to do with a particular
/// project. `None` if the OS cache directory cannot be determined, in which
/// case caching is simply skipped.
fn cache_file_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("meson-jll").join("yggdrasil-packages.json"))
}

/// Reads a cached package list from `path`, returning it only if it was
/// computed from the same commit as `sha`. Any failure to read or parse the
/// cache (missing file, corrupt JSON, a stale format) is treated as a plain
/// cache miss rather than an error, since the cache is a pure optimization.
fn read_cached_package_list(sha: &str, path: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(path).ok()?;
    let cached: CachedPackageList = serde_json::from_str(&text).ok()?;
    (cached.sha == sha).then_some(cached.names)
}

/// Best-effort cache write. Failing to create the cache directory or write
/// the file is silently ignored, since the cache only ever speeds up a
/// later call and must never turn into a hard error on its own.
fn write_cached_package_list(sha: &str, names: &[String], path: &Path) {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let cached = CachedPackageList {
        sha: sha.to_string(),
        names: names.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&cached) {
        let _ = fs::write(path, text);
    }
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
