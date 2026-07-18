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

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use flate2::read::GzDecoder;

use crate::cache;
use crate::error::{Error, Result};
use crate::git;

/// A place to fetch a JLL's metadata files from, by path relative to the
/// repository root (for example `Project.toml` or
/// `src/wrappers/x86_64-linux-gnu.jl`).
pub trait Source {
    fn fetch(&self, relative_path: &str) -> Result<String>;
}

/// Fetches files from a GitHub repository at a fixed git ref (a tag or
/// branch name).
///
/// A JLL's metadata is several files ([`crate::jll::load`] reads
/// `Project.toml`, `Artifacts.toml`, and one wrapper script per supported
/// platform, which can be a dozen or more for a JLL with many platforms).
/// Fetching each individually was one HTTP request per file. Instead, the
/// whole repository at that ref is downloaded once, as the same gzipped
/// tarball GitHub's own "Source code" release links point to, and every
/// [`Source::fetch`] call after that is answered from the already
/// downloaded, already decompressed contents. This turns what used to be a
/// request per file into one request per repository, regardless of how
/// many files end up being read from it.
///
/// The downloaded contents are also cached on disk, keyed by the commit
/// `git_ref` currently points at (see [`Self::load_archive`]), so asking
/// for the same tag again, in a later `meson-jll` invocation entirely,
/// never re-downloads it. A tag's commit never changes, so this cache
/// entry is kept forever once written; a branch's commit can, so a moving
/// branch (`main`, used for a `--url` source with no version pinned) is
/// still re-resolved and, if it moved, re-downloaded every time.
pub struct GithubSource {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
    /// The archive's files, keyed by path relative to the repository root
    /// with forward slashes, downloaded and decompressed the first time
    /// [`Source::fetch`] is called, and reused for every call after that.
    archive: RefCell<Option<HashMap<String, String>>>,
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
            archive: RefCell::new(None),
        }
    }

    /// Returns this repository's files at `self.git_ref`, from the on-disk
    /// cache if a previous call (in this run or an earlier one) already
    /// downloaded that exact commit, or by downloading it otherwise.
    ///
    /// Resolving `self.git_ref` to a commit is itself one small git
    /// operation ([`git::ls_remote_sha`]), not an HTTP request, so checking
    /// the cache never costs a full request even on a cache hit.
    fn load_archive(&self) -> Result<HashMap<String, String>> {
        let url = format!("https://github.com/{}/{}.git", self.owner, self.repo);
        let commit = git::ls_remote_sha(&url, &self.git_ref)?;

        let cache_path = archive_cache_path(&self.owner, &self.repo, &commit);
        if let Some(path) = &cache_path {
            if let Some(files) = cache::read_json(path) {
                return Ok(files);
            }
        }

        let files = self.download_archive()?;
        if let Some(path) = &cache_path {
            cache::write_json(path, &files);
        }
        Ok(files)
    }

    /// Downloads and unpacks this repository's gzipped tarball at
    /// `self.git_ref`, in memory, into a map of relative path to file
    /// contents.
    ///
    /// GitHub wraps every archive in one top-level directory (its exact
    /// name is not documented and not relied on here), so each entry's
    /// first path component is dropped rather than matched against an
    /// expected name. A directory entry, or a file that does not decode as
    /// UTF-8 text, is skipped rather than treated as an error: every file
    /// this tool actually reads from a JLL repository is plain text, so a
    /// stray binary or symlink entry elsewhere in the repository is simply
    /// not something any `fetch` call will ever ask for.
    fn download_archive(&self) -> Result<HashMap<String, String>> {
        let url = format!(
            "https://github.com/{}/{}/archive/{}.tar.gz",
            self.owner, self.repo, self.git_ref
        );
        let response = ureq::get(&url)
            .set("User-Agent", "meson-jll")
            .call()
            .map_err(|source| Error::Fetch {
                url: url.clone(),
                source: Box::new(source),
            })?;

        let decoder = GzDecoder::new(response.into_reader());
        let mut archive = tar::Archive::new(decoder);
        let entries = archive.entries().map_err(|source| Error::ReadArchive {
            url: url.clone(),
            source,
        })?;

        let mut files = HashMap::new();
        for entry in entries {
            let mut entry = entry.map_err(|source| Error::ReadArchive {
                url: url.clone(),
                source,
            })?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path().map_err(|source| Error::ReadArchive {
                url: url.clone(),
                source,
            })?;
            let relative_path = path
                .components()
                .skip(1)
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let mut contents = String::new();
            if entry.read_to_string(&mut contents).is_ok() {
                files.insert(relative_path, contents);
            }
        }
        Ok(files)
    }
}

/// Where a `(owner, repo, commit)` archive is cached, in the OS's own cache
/// directory, one file per commit rather than the single shared file
/// [`crate::registry::yggdrasil_package_names`] uses, since many distinct
/// commits are all worth keeping at once here (unlike Yggdrasil's package
/// list, where only the current state is ever useful). `None` if the OS
/// cache directory cannot be determined, in which case caching is simply
/// skipped.
fn archive_cache_path(owner: &str, repo: &str, commit: &str) -> Option<PathBuf> {
    cache::cache_dir().map(|dir| {
        dir.join("archives")
            .join(owner)
            .join(repo)
            .join(format!("{commit}.json"))
    })
}

impl Source for GithubSource {
    fn fetch(&self, relative_path: &str) -> Result<String> {
        if self.archive.borrow().is_none() {
            let files = self.load_archive()?;
            *self.archive.borrow_mut() = Some(files);
        }
        self.archive
            .borrow()
            .as_ref()
            .expect("just populated above")
            .get(relative_path)
            .cloned()
            .ok_or_else(|| Error::MissingArchiveEntry {
                owner: self.owner.clone(),
                repo: self.repo.clone(),
                git_ref: self.git_ref.clone(),
                path: relative_path.to_string(),
            })
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
