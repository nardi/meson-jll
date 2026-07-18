//! Listing a repository's tags and ref commits over git's own wire
//! protocol, rather than the GitHub REST API or a `git` subprocess.
//!
//! `git ls-remote` (and the first step of `git clone`/`fetch`) works by
//! sending one unauthenticated `GET` to `<repository>/info/refs?service=
//! git-upload-pack` and parsing the "smart HTTP" ref advertisement it gets
//! back (see gitprotocol-http(5) and gitprotocol-pack(5)). That request is
//! not subject to `api.github.com`'s 60-requests-per-hour unauthenticated
//! rate limit, a limit ordinary, per-package use of this tool (one lookup
//! per JLL dependency) ran into.
//!
//! This module makes that same request directly with `ureq` (already a
//! dependency) instead of shelling out to the `git` binary. Measured
//! against GitHub, shelling out took roughly twice as long per call as the
//! request underneath it alone (~500-600ms against ~300-350ms), the
//! difference being `git`'s own process startup and protocol negotiation on
//! top of the request. Making the request directly also lets `ureq`'s
//! default connection pool reuse one warm connection across calls in the
//! same run, and removes `git` itself as a runtime dependency of this tool.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};

use crate::error::{Error, Result};

/// Fetches and parses `url`'s full ref advertisement: every ref the
/// repository has, each paired with the commit it points at, in whatever
/// order the server sent them (not creation order).
///
/// A repository that does not exist, or one this request has no read
/// access to, gets a failing HTTP status with a "not found" body; GitHub
/// does not distinguish the two cases, so neither does this, reading as an
/// empty advertisement (the same outcome as a real, empty repository)
/// rather than an error. Any other failing status, or a body that does not
/// parse as a ref advertisement at all, is a real error.
fn fetch_ref_advertisement(url: &str) -> Result<Vec<(String, String)>> {
    let advertisement_url = format!("{url}/info/refs?service=git-upload-pack");
    let response = match ureq::get(&advertisement_url)
        .set("User-Agent", "meson-jll")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            if body.to_lowercase().contains("not found") {
                return Ok(Vec::new());
            }
            return Err(Error::FetchRefsStatus {
                url: advertisement_url,
                status,
                body,
            });
        }
        Err(other) => {
            return Err(Error::Fetch {
                url: advertisement_url,
                source: Box::new(other),
            })
        }
    };

    let mut body = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|source| Error::ReadResponseBody {
            url: advertisement_url.clone(),
            source,
        })?;

    parse_ref_advertisement(&body).ok_or(Error::ParseRefAdvertisement {
        url: advertisement_url,
    })
}

/// Parses git's smart-HTTP pkt-line ref advertisement into (ref name,
/// commit sha) pairs.
///
/// A pkt-line is a 4-hex-digit length, of the whole line including that
/// header, followed by that many bytes of content; a length of `0000` is a
/// "flush" separator rather than a real line. The first real line is a
/// service header (`# service=git-upload-pack`), which is skipped along
/// with everything up to the flush that follows it; every line after that
/// is `<sha> <ref-name>`, with the very first one additionally carrying a
/// NUL-separated capabilities list that is stripped off.
fn parse_ref_advertisement(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    let mut refs = Vec::new();
    let mut position = 0;
    let mut past_service_header = false;

    while position + 4 <= bytes.len() {
        let length_hex = std::str::from_utf8(&bytes[position..position + 4]).ok()?;
        let length = usize::from_str_radix(length_hex, 16).ok()?;
        position += 4;

        if length == 0 {
            past_service_header = true;
            continue;
        }
        let content_length = length.checked_sub(4)?;
        let content = bytes.get(position..position + content_length)?;
        position += content_length;

        if !past_service_header {
            continue;
        }

        let line = std::str::from_utf8(content).ok()?.trim_end_matches('\n');
        let line = line.split('\0').next()?;
        let (sha, reference) = line.split_once(' ')?;
        refs.push((reference.to_string(), sha.to_string()));
    }

    Some(refs)
}

/// Lists the tags of a remote repository, each paired with the commit it
/// points at.
///
/// A single call already carries every tag's commit, so a caller that is
/// about to ask [`ls_remote_sha`] for one of these same `(url, tag)` pairs
/// should [`remember_sha`] it first instead, turning that second lookup
/// into a memo hit rather than another request.
pub fn ls_remote_tags(url: &str) -> Result<Vec<(String, String)>> {
    let refs = fetch_ref_advertisement(url)?;
    Ok(extract_tags(&refs))
}

/// Pulls (tag name, commit sha) pairs out of a full ref advertisement,
/// sorted by name and deduplicated.
///
/// An annotated tag is advertised twice: once as the tag object itself,
/// and once dereferenced (suffixed `^{}`) to the commit it actually points
/// at. The dereferenced commit is the one wanted, matching
/// [`find_ref_sha`], regardless of which of the two is seen first.
fn extract_tags(refs: &[(String, String)]) -> Vec<(String, String)> {
    let mut shas_by_name: HashMap<String, String> = HashMap::new();
    for (reference, sha) in refs {
        let Some(name) = reference.strip_prefix("refs/tags/") else {
            continue;
        };
        match name.strip_suffix("^{}") {
            Some(dereferenced_name) => {
                shas_by_name.insert(dereferenced_name.to_string(), sha.clone());
            }
            None => {
                shas_by_name
                    .entry(name.to_string())
                    .or_insert_with(|| sha.clone());
            }
        }
    }
    let mut tags: Vec<(String, String)> = shas_by_name.into_iter().collect();
    tags.sort_by(|left, right| left.0.cmp(&right.0));
    tags
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
/// version), so skipping a repeat one is worth a small in-memory cache even
/// though the answer is not persisted past this run, unlike the on-disk
/// archive cache this feeds into.
pub fn ls_remote_sha(url: &str, reference: &str) -> Result<String> {
    let key = (url.to_string(), reference.to_string());
    if let Some(sha) = lock_sha_cache().get(&key) {
        return Ok(sha.clone());
    }

    let refs = fetch_ref_advertisement(url)?;
    let sha = find_ref_sha(&refs, reference).ok_or_else(|| Error::GitRefNotFound {
        url: url.to_string(),
        reference: reference.to_string(),
    })?;

    lock_sha_cache().insert(key, sha.clone());
    Ok(sha)
}

/// Finds `reference`'s commit among a full ref advertisement. `reference`
/// may be a bare name, the same shorthand `git ls-remote <url> <name>`
/// accepts, matched in turn against an annotated tag's dereferenced commit,
/// a plain tag, a branch, and finally the name as given outright. The first
/// of these that matches wins.
fn find_ref_sha(refs: &[(String, String)], reference: &str) -> Option<String> {
    let candidates = [
        format!("refs/tags/{reference}^{{}}"),
        format!("refs/tags/{reference}"),
        format!("refs/heads/{reference}"),
        reference.to_string(),
    ];
    candidates.iter().find_map(|candidate| {
        refs.iter()
            .find(|(name, _)| name == candidate)
            .map(|(_, sha)| sha.clone())
    })
}

/// Records that `reference` on `url` is already known to point at `sha`,
/// without making a request, so a later [`ls_remote_sha`] call for this
/// same pair is a memo hit instead of another round trip. Meant for a
/// caller that already has this answer as a side effect of something else
/// it just did (see [`ls_remote_tags`]).
pub fn remember_sha(url: &str, reference: &str, sha: &str) {
    lock_sha_cache().insert((url.to_string(), reference.to_string()), sha.to_string());
}

/// Locks the process-lifetime memo table [`ls_remote_sha`] reads and writes.
/// A poisoned lock (a previous holder panicked mid-update) is recovered
/// rather than propagated, since losing this cache costs at most a repeat
/// request, never incorrect data.
fn lock_sha_cache() -> std::sync::MutexGuard<'static, HashMap<(String, String), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one pkt-line (the 4-hex-digit length header, including
    /// itself, followed by `content`), the same shape a real ref
    /// advertisement is made of, so tests never need to compute lengths by
    /// hand.
    fn pkt_line(content: &str) -> String {
        format!("{:04x}{content}", content.len() + 4)
    }

    fn sample_advertisement() -> String {
        let mut advertisement = String::new();
        advertisement.push_str(&pkt_line("# service=git-upload-pack\n"));
        advertisement.push_str("0000");
        advertisement.push_str(&pkt_line(
            "abc123 HEAD\0multi_ack thin-pack side-band-64k\n",
        ));
        advertisement.push_str(&pkt_line("abc123 refs/heads/main\n"));
        advertisement.push_str(&pkt_line("def456 refs/tags/ExampleThing-v1.2.3+0\n"));
        advertisement.push_str("0000");
        advertisement
    }

    #[test]
    fn parses_a_ref_advertisement() {
        let refs = parse_ref_advertisement(sample_advertisement().as_bytes()).unwrap();
        assert_eq!(
            refs,
            vec![
                ("HEAD".to_string(), "abc123".to_string()),
                ("refs/heads/main".to_string(), "abc123".to_string()),
                (
                    "refs/tags/ExampleThing-v1.2.3+0".to_string(),
                    "def456".to_string()
                ),
            ]
        );
    }

    #[test]
    fn truncated_input_fails_to_parse() {
        let mut advertisement = sample_advertisement();
        advertisement.truncate(advertisement.len() - 5);
        assert!(parse_ref_advertisement(advertisement.as_bytes()).is_none());
    }

    #[test]
    fn extracts_lightweight_tag_refs() {
        let refs = vec![
            (
                "refs/tags/ExampleThing-v1.2.3+0".to_string(),
                "abc123".to_string(),
            ),
            (
                "refs/tags/ExampleThing-v1.3.0+0".to_string(),
                "def456".to_string(),
            ),
            ("refs/heads/main".to_string(), "ghi789".to_string()),
        ];
        assert_eq!(
            extract_tags(&refs),
            vec![
                ("ExampleThing-v1.2.3+0".to_string(), "abc123".to_string()),
                ("ExampleThing-v1.3.0+0".to_string(), "def456".to_string()),
            ]
        );
    }

    #[test]
    fn an_annotated_tag_reports_the_dereferenced_commit_not_the_tag_object() {
        let refs = vec![
            (
                "refs/tags/ExampleThing-v1.2.3+0".to_string(),
                "tagobject123".to_string(),
            ),
            (
                "refs/tags/ExampleThing-v1.2.3+0^{}".to_string(),
                "commit456".to_string(),
            ),
        ];
        assert_eq!(
            extract_tags(&refs),
            vec![("ExampleThing-v1.2.3+0".to_string(), "commit456".to_string())]
        );
    }

    #[test]
    fn no_tags_is_an_empty_list() {
        assert_eq!(extract_tags(&[]), Vec::<(String, String)>::new());
    }

    #[test]
    fn finds_a_branch_by_bare_name() {
        let refs = vec![("refs/heads/main".to_string(), "abc123".to_string())];
        assert_eq!(find_ref_sha(&refs, "main"), Some("abc123".to_string()));
    }

    #[test]
    fn finds_an_annotated_tag_by_bare_name_preferring_the_commit() {
        let refs = vec![
            (
                "refs/tags/ExampleThing-v1.2.3+0".to_string(),
                "tagobject123".to_string(),
            ),
            (
                "refs/tags/ExampleThing-v1.2.3+0^{}".to_string(),
                "commit456".to_string(),
            ),
        ];
        assert_eq!(
            find_ref_sha(&refs, "ExampleThing-v1.2.3+0"),
            Some("commit456".to_string())
        );
    }

    #[test]
    fn no_matching_ref_is_none() {
        assert_eq!(find_ref_sha(&[], "does-not-exist"), None);
    }
}
