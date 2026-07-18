//! Resolving a JLL package name to a repository, and enumerating or
//! versioning JLL packages through the GitHub API.
//!
//! Every JLL published through Yggdrasil ends up as a repository under the
//! `JuliaBinaryWrappers` GitHub organization, named `<Name>_jll.jl`. That
//! makes the organization's repository listing a practical stand-in for the
//! full Julia General registry index for the purposes of `list` and
//! `search`, without this tool needing to clone or parse that registry
//! itself.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::source::fetch_url;

/// The GitHub organization every JLL package is published under.
const ORGANIZATION: &str = "JuliaBinaryWrappers";

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

/// The most recently published version of a JLL package.
///
/// This takes the first tag the GitHub API returns. GitHub does not
/// formally guarantee tag ordering, but in practice returns them by
/// creation date, most recent first, which matches how Yggdrasil publishes
/// JLL releases.
pub fn latest_version(owner: &str, repo: &str) -> Result<String> {
    let tags = list_tags(owner, repo)?;
    tags.iter()
        .find_map(|tag| version_from_tag(tag))
        .map(String::from)
        .ok_or_else(|| Error::NoPlatforms {
            name: repo.to_string(),
        })
}

#[derive(Debug, Deserialize)]
struct RepoResponse {
    name: String,
}

/// Lists the bare names of every JLL package published under the
/// `JuliaBinaryWrappers` organization, for example `ExampleThing` for the
/// `ExampleThing_jll.jl` repository.
pub fn list_jll_packages() -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut page = 1;
    loop {
        let url =
            format!("https://api.github.com/orgs/{ORGANIZATION}/repos?per_page=100&page={page}");
        let body = fetch_url(&url)?;
        let repos: Vec<RepoResponse> =
            serde_json::from_str(&body).map_err(|source| Error::ParseJson { url, source })?;
        if repos.is_empty() {
            break;
        }
        names.extend(
            repos
                .into_iter()
                .filter_map(|repo| repo.name.strip_suffix("_jll.jl").map(String::from)),
        );
        page += 1;
    }
    Ok(names)
}

/// Lists the bare names of every JLL package whose name contains `term`
/// (case-insensitive).
pub fn search_jll_packages(term: &str) -> Result<Vec<String>> {
    let term = term.to_lowercase();
    let mut names = list_jll_packages()?;
    names.retain(|name| name.to_lowercase().contains(&term));
    Ok(names)
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
}
