//! Where a JLL's metadata files are fetched from.
//!
//! By default a JLL's metadata comes straight from its GitHub repository at
//! a given tag ([`GithubSource`]). Passing `--url` to `install` swaps that
//! for a caller-supplied location instead ([`CustomSource`]), which may be
//! another GitHub repository or a local directory following the same
//! layout. Both are small enough that a plain trait with two
//! implementations is all this needs. Call sites pick the implementation
//! they want and call [`Source::fetch`] directly on it, so which one is
//! used is resolved statically rather than through a boxed trait object.

use std::fs;
use std::path::PathBuf;

use crate::error::{Error, Result};

/// A place to fetch a JLL's metadata files from, by path relative to the
/// repository root (for example `Project.toml` or
/// `src/wrappers/x86_64-linux-gnu.jl`).
pub trait Source {
    fn fetch(&self, relative_path: &str) -> Result<String>;
}

/// Fetches files from a GitHub repository's raw content server, at a fixed
/// git ref (a tag or branch name).
pub struct GithubSource {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
}

impl GithubSource {
    pub fn new(
        owner: impl Into<String>,
        repo: impl Into<String>,
        git_ref: impl Into<String>,
    ) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
            git_ref: git_ref.into(),
        }
    }
}

impl Source for GithubSource {
    fn fetch(&self, relative_path: &str) -> Result<String> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            self.owner, self.repo, self.git_ref, relative_path
        );
        fetch_url(&url)
    }
}

/// A caller-supplied `--url` source: either another GitHub repository or a
/// local directory, chosen by whether the given string looks like a GitHub
/// URL.
pub struct CustomSource {
    inner: CustomSourceKind,
}

enum CustomSourceKind {
    Local(PathBuf),
    Github(GithubSource),
}

impl CustomSource {
    /// Builds a source from the value passed to `--url`, using `git_ref`
    /// when it turns out to point at a GitHub repository.
    pub fn parse(url_or_path: &str, git_ref: &str) -> Self {
        let github_prefixes = ["https://github.com/", "http://github.com/"];
        for prefix in github_prefixes {
            if let Some(rest) = url_or_path.strip_prefix(prefix) {
                let rest = rest.trim_end_matches(".git").trim_end_matches('/');
                let mut parts = rest.splitn(2, '/');
                let owner = parts.next().unwrap_or_default().to_string();
                let repo = parts.next().unwrap_or_default().to_string();
                return Self {
                    inner: CustomSourceKind::Github(GithubSource::new(owner, repo, git_ref)),
                };
            }
        }
        Self {
            inner: CustomSourceKind::Local(PathBuf::from(url_or_path)),
        }
    }
}

impl Source for CustomSource {
    fn fetch(&self, relative_path: &str) -> Result<String> {
        match &self.inner {
            CustomSourceKind::Local(base) => {
                let path = base.join(relative_path);
                fs::read_to_string(&path).map_err(|source| Error::ReadLocalFile { path, source })
            }
            CustomSourceKind::Github(source) => source.fetch(relative_path),
        }
    }
}

/// Performs a blocking HTTP GET and returns the response body as text.
///
/// Sets a `User-Agent` header on every request, since GitHub's API rejects
/// requests that do not have one.
pub(crate) fn fetch_url(url: &str) -> Result<String> {
    let response = ureq::get(url)
        .set("User-Agent", "meson-jll")
        .call()
        .map_err(|source| Error::Fetch {
            url: url.to_string(),
            source: Box::new(source),
        })?;
    response
        .into_string()
        .map_err(|source| Error::ReadResponseBody {
            url: url.to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_github_url() {
        let source = CustomSource::parse("https://github.com/me/MyThing_jll.jl", "main");
        match source.inner {
            CustomSourceKind::Github(github) => {
                assert_eq!(github.owner, "me");
                assert_eq!(github.repo, "MyThing_jll.jl");
                assert_eq!(github.git_ref, "main");
            }
            CustomSourceKind::Local(_) => panic!("expected a Github source"),
        }
    }

    #[test]
    fn treats_anything_else_as_a_local_path() {
        let source = CustomSource::parse("./local/MyThing_jll.jl", "main");
        match source.inner {
            CustomSourceKind::Local(path) => {
                assert_eq!(path, PathBuf::from("./local/MyThing_jll.jl"));
            }
            CustomSourceKind::Github(_) => panic!("expected a Local source"),
        }
    }
}
