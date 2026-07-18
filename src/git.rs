//! Listing a repository's tags through git's own protocol rather than the
//! GitHub REST API.
//!
//! `git ls-remote --tags` answers "what tags does this repository have" the
//! same way the REST API's tags endpoint does, but over git's smart HTTP
//! transport instead of `api.github.com`, which is not subject to that
//! API's 60-requests-per-hour unauthenticated rate limit, a limit ordinary,
//! per-package use of this tool (one lookup per JLL dependency) ran into.
//! This needs nothing beyond the system `git` binary, already a reasonable
//! dependency for a tool that writes Meson wrap files (Meson's own wrap
//! system shells out to `git` for VCS subprojects).

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use crate::error::{Error, Result};

/// Runs `git` with `args` and returns its stdout as text.
///
/// `GIT_TERMINAL_PROMPT=0` is set so a private or missing repository fails
/// immediately with a normal error instead of `git` hanging on an
/// interactive credential prompt. A `git` binary that could not even be
/// started becomes [`Error::RunGit`]; a git command that ran but exited
/// with a failure becomes [`Error::GitFailed`], carrying its stderr so the
/// underlying reason (a missing repository, a network failure) is still
/// visible to the caller.
///
/// Each call is its own `git` process and its own HTTPS connection, so the
/// connection and TLS handshake (measured at a flat 500-600ms against
/// GitHub, regardless of what is actually being asked for) dominates the
/// cost of every call. This is why callers that might plausibly ask the
/// same question twice in one run, like [`ls_remote_sha`], memoize rather
/// than calling through here again.
fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|source| Error::RunGit {
            args: args.iter().map(|argument| argument.to_string()).collect(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::GitFailed {
            args: args.iter().map(|argument| argument.to_string()).collect(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Lists the tags of a remote repository, each paired with the commit it
/// points at, in whatever order the remote's own ref advertisement returns
/// them. This is not creation order, so a caller that cares about "the
/// latest version" compares parsed versions rather than relying on it.
///
/// A single `git ls-remote --tags` already carries every tag's commit, so a
/// caller that is about to ask [`ls_remote_sha`] for one of these same
/// `(url, tag)` pairs should [`remember_sha`] it first instead, turning
/// that second lookup into a memo hit rather than another round trip.
pub fn ls_remote_tags(url: &str) -> Result<Vec<(String, String)>> {
    let output = run(&["ls-remote", "--tags", url])?;
    Ok(parse_tag_refs(&output))
}

/// Pulls (tag name, commit sha) pairs out of `git ls-remote --tags`'s
/// output (lines of `<sha>\trefs/tags/<name>`), sorted by name and
/// deduplicated.
///
/// An annotated tag is advertised twice: once as the tag object itself,
/// and once dereferenced (suffixed `^{}`) to the commit it actually points
/// at. The dereferenced commit is the one wanted, matching
/// [`parse_ref_sha`], regardless of which of the two lines is seen first.
fn parse_tag_refs(output: &str) -> Vec<(String, String)> {
    let mut shas_by_name: HashMap<String, String> = HashMap::new();
    for line in output.lines() {
        let mut columns = line.split('\t');
        let (Some(sha), Some(reference)) = (columns.next(), columns.next()) else {
            continue;
        };
        let Some(name) = reference.strip_prefix("refs/tags/") else {
            continue;
        };
        match name.strip_suffix("^{}") {
            Some(dereferenced_name) => {
                shas_by_name.insert(dereferenced_name.to_string(), sha.to_string());
            }
            None => {
                shas_by_name
                    .entry(name.to_string())
                    .or_insert_with(|| sha.to_string());
            }
        }
    }
    let mut tags: Vec<(String, String)> = shas_by_name.into_iter().collect();
    tags.sort_by(|left, right| left.0.cmp(&right.0));
    tags
}

/// `true` if a failed git command's stderr looks like "no such repository",
/// as opposed to some other failure (a network error, an unrelated git
/// failure).
pub fn looks_like_missing_repository(stderr: &str) -> bool {
    let lowercase = stderr.to_lowercase();
    lowercase.contains("not found") || lowercase.contains("could not find")
}

/// The commit a tag or branch on a remote currently points at, used to key
/// a content cache against (see `crate::source::GithubSource`): the same
/// commit always has the same file contents, so caching by commit rather
/// than by ref name still gets a fresh answer if a branch (unlike a tag)
/// later moves to point somewhere else.
///
/// Memoized for the lifetime of the process: the same `(url, reference)` is
/// genuinely asked for twice in one run in practice (once resolving a
/// package's version, once regenerating its wrap set from the resolved
/// version), and since each call is its own ~500ms round trip (see [`run`]),
/// skipping a repeat one is worth a small in-memory cache even though the
/// answer is not persisted past this run, unlike the on-disk archive cache
/// this feeds into.
pub fn ls_remote_sha(url: &str, reference: &str) -> Result<String> {
    let key = (url.to_string(), reference.to_string());
    if let Some(sha) = lock_sha_cache().get(&key) {
        return Ok(sha.clone());
    }

    let output = run(&["ls-remote", url, reference])?;
    let sha = parse_ref_sha(&output).ok_or_else(|| Error::GitRefNotFound {
        url: url.to_string(),
        reference: reference.to_string(),
    })?;

    lock_sha_cache().insert(key, sha.clone());
    Ok(sha)
}

/// Records that `reference` on `url` is already known to point at `sha`,
/// without asking git anything, so a later [`ls_remote_sha`] call for this
/// same pair is a memo hit instead of another ~500ms round trip. Meant for
/// a caller that already has this answer as a side effect of something
/// else it just did (see [`ls_remote_tags`]).
pub fn remember_sha(url: &str, reference: &str, sha: &str) {
    lock_sha_cache().insert((url.to_string(), reference.to_string()), sha.to_string());
}

/// Locks the process-lifetime memo table [`ls_remote_sha`] reads and writes.
/// A poisoned lock (a previous holder panicked mid-update) is recovered
/// rather than propagated, since losing this cache costs at most a repeat
/// `git` call, never incorrect data.
fn lock_sha_cache() -> std::sync::MutexGuard<'static, HashMap<(String, String), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Picks the commit SHA out of `git ls-remote`'s output for a single ref.
/// An annotated tag advertises both the tag object's own SHA and,
/// dereferenced (suffixed `^{}`), the commit it points at. The dereferenced
/// commit is the one wanted, and is preferred if present; a lightweight tag
/// or a branch has no such line, and its own SHA already is a commit.
fn parse_ref_sha(output: &str) -> Option<String> {
    let mut first_seen: Option<String> = None;
    for line in output.lines() {
        let mut columns = line.split('\t');
        let sha = columns.next()?;
        let reference = columns.next()?;
        if reference.ends_with("^{}") {
            return Some(sha.to_string());
        }
        first_seen.get_or_insert_with(|| sha.to_string());
    }
    first_seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lightweight_tag_refs() {
        let output = "\
abc123\trefs/tags/ExampleThing-v1.2.3+0
def456\trefs/tags/ExampleThing-v1.3.0+0
";
        assert_eq!(
            parse_tag_refs(output),
            vec![
                ("ExampleThing-v1.2.3+0".to_string(), "abc123".to_string()),
                ("ExampleThing-v1.3.0+0".to_string(), "def456".to_string()),
            ]
        );
    }

    #[test]
    fn an_annotated_tag_reports_the_dereferenced_commit_not_the_tag_object() {
        let output = "\
tagobject123\trefs/tags/ExampleThing-v1.2.3+0
commit456\trefs/tags/ExampleThing-v1.2.3+0^{}
";
        assert_eq!(
            parse_tag_refs(output),
            vec![("ExampleThing-v1.2.3+0".to_string(), "commit456".to_string())]
        );
    }

    #[test]
    fn empty_output_is_no_tags() {
        assert_eq!(parse_tag_refs(""), Vec::<(String, String)>::new());
    }

    #[test]
    fn recognises_a_repository_not_found_message() {
        assert!(looks_like_missing_repository(
            "fatal: repository 'https://github.com/owner/repo.git/' not found"
        ));
    }

    #[test]
    fn does_not_mistake_a_network_failure_for_a_missing_repository() {
        assert!(!looks_like_missing_repository(
            "fatal: unable to access: connection timed out"
        ));
    }

    #[test]
    fn a_lightweight_tag_or_branch_sha_is_used_directly() {
        let output = "abc123\trefs/heads/main\n";
        assert_eq!(parse_ref_sha(output), Some("abc123".to_string()));
    }

    #[test]
    fn an_annotated_tag_prefers_the_dereferenced_commit() {
        let output = "\
tagobject123\trefs/tags/ExampleThing-v1.2.3+0
commit456\trefs/tags/ExampleThing-v1.2.3+0^{}
";
        assert_eq!(parse_ref_sha(output), Some("commit456".to_string()));
    }

    #[test]
    fn no_matching_ref_is_none() {
        assert_eq!(parse_ref_sha(""), None);
    }
}
