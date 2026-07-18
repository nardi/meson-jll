//! Julia-style version numbers and compat specifiers.
//!
//! JLL versions and their `[compat]` bounds follow Julia's own semver-like
//! scheme, not Cargo's. A `semver`-style crate is avoided on purpose for two
//! reasons. First, JLL versions carry a build number (`7.12.1+0`) that is
//! not decoration the way plain SemVer build metadata is: Yggdrasil bumps it
//! on a rebuild of the same upstream release, so `7.8.3+2` really is a later
//! version than `7.8.3+1`, and comparisons need to take it into account.
//! Second, Julia's caret and tilde compat ranges follow their own
//! "leftmost nonzero component" rule (see [`CompatSpecifier::parse`]), which
//! does not line up with Cargo's caret rule closely enough to reuse.
//!
//! This mirrors the compat parsing in the sibling `hatch_jll` Python
//! project, translated to Rust.

use crate::error::{Error, Result};

/// A Julia-style version: `major.minor.patch`, plus the build number after
/// a `+`, defaulting to zero when absent. Ordering compares all four
/// components in that order, so `7.12.1+0 < 7.12.1+1 < 7.13.0+0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub build: u64,
}

impl Version {
    /// Parses a version string such as `7.12.1+0`, `1.2.3`, or `1.11`.
    /// Missing trailing components (minor, patch, build) default to zero.
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = || Error::ParseVersion {
            text: text.to_string(),
        };

        let (numbered_part, build_part) = match text.split_once('+') {
            Some((numbered, build)) => (numbered, Some(build)),
            None => (text, None),
        };

        let mut components = numbered_part.split('.');
        let mut next_component = || -> Result<u64> {
            match components.next() {
                Some(part) => part.parse().map_err(|_| invalid()),
                None => Ok(0),
            }
        };
        let major = next_component()?;
        let minor = next_component()?;
        let patch = next_component()?;

        let build = match build_part {
            Some(build) => build.parse().map_err(|_| invalid())?,
            None => 0,
        };

        Ok(Self {
            major,
            minor,
            patch,
            build,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}+{}",
            self.major, self.minor, self.patch, self.build
        )
    }
}

/// A half-open version range `[lower, upper)`, or unbounded above when
/// `upper` is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    pub lower: Version,
    pub upper: Option<Version>,
}

impl VersionRange {
    /// A range that accepts every version.
    pub fn unbounded() -> Self {
        Self {
            lower: Version::default(),
            upper: None,
        }
    }

    pub fn contains(&self, version: Version) -> bool {
        version >= self.lower && self.upper.is_none_or(|upper| version < upper)
    }
}

/// A Julia compat specifier, for example `"1.2, 2"`, parsed into the union
/// of ranges it allows. A version satisfies the specifier if it falls in
/// any one of them, matching the comma-as-or meaning `Project.toml` compat
/// entries use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatSpecifier(Vec<VersionRange>);

impl CompatSpecifier {
    /// Parses a full compat specifier such as `"1.2, 2"`.
    pub fn parse(text: &str) -> Self {
        Self(text.split(',').map(parse_compat_token).collect())
    }

    /// A specifier that accepts every version. Used when a package declares
    /// no compat entry for a dependency at all.
    pub fn unbounded() -> Self {
        Self(vec![VersionRange::unbounded()])
    }

    pub fn contains(&self, version: Version) -> bool {
        self.0.iter().any(|range| range.contains(version))
    }
}

/// Parses the numeric components of a bound string such as `"1.2.3"`,
/// `"1.2"`, `"1"`, or `""`/`"*"` (empty, meaning no components at all).
/// Returns `None` if a component is not a plain non-negative integer, so
/// the caller can fall back to treating the whole token as unbounded.
fn parse_components(text: &str) -> Option<Vec<u64>> {
    if text.is_empty() || text == "*" {
        return Some(Vec::new());
    }
    text.split('.').map(|part| part.parse().ok()).collect()
}

/// Builds the lower bound of a range from its given components, padding
/// any missing trailing component with zero.
fn lower_from_components(components: &[u64]) -> Version {
    let mut padded = [0u64; 3];
    for (slot, value) in padded.iter_mut().zip(components) {
        *slot = *value;
    }
    Version {
        major: padded[0],
        minor: padded[1],
        patch: padded[2],
        build: 0,
    }
}

/// Builds the exclusive upper bound one step past the last component given,
/// with every later component reset to zero. `"1.2"` gives `1.3.0`, and
/// `"1"` gives `2.0.0`. Unbounded (no components given) when `components`
/// is empty.
fn upper_exclusive_from_components(components: &[u64]) -> Option<Version> {
    if components.is_empty() {
        return None;
    }
    let mut padded = [0u64; 3];
    for (slot, value) in padded.iter_mut().zip(components) {
        *slot = *value;
    }
    let bump_index = components.len().min(3) - 1;
    padded[bump_index] += 1;
    for slot in padded.iter_mut().skip(bump_index + 1) {
        *slot = 0;
    }
    Some(Version {
        major: padded[0],
        minor: padded[1],
        patch: padded[2],
        build: 0,
    })
}

/// Expands a caret bound, the default when no prefix is given.
///
/// Julia's caret ranges use the same "leftmost nonzero component" rule as
/// npm's `^` ranges: the range stays compatible up to (but excluding) the
/// next value that would change the leftmost nonzero component given.
/// `^1.2.3` allows up to `2.0.0`, `^0.2.3` allows up to `0.3.0`, and
/// `^0.0.3` allows only up to `0.0.4`, treating `0.x` releases as unstable
/// the same way npm does.
fn caret_range(bound: &str) -> Option<VersionRange> {
    let components = parse_components(bound)?;
    if components.is_empty() {
        return Some(VersionRange::unbounded());
    }
    let lower = lower_from_components(&components);

    let mut padded = [0u64; 3];
    for (slot, value) in padded.iter_mut().zip(&components) {
        *slot = *value;
    }
    let bump_index = components
        .iter()
        .position(|component| *component != 0)
        .unwrap_or(components.len().min(3) - 1);
    padded[bump_index] += 1;
    for slot in padded.iter_mut().skip(bump_index + 1) {
        *slot = 0;
    }
    let upper = Version {
        major: padded[0],
        minor: padded[1],
        patch: padded[2],
        build: 0,
    };

    Some(VersionRange {
        lower,
        upper: Some(upper),
    })
}

/// Expands a tilde bound. `~1.2.3` and `~1.2` both allow patch level
/// updates within `1.2.x`. `~1` allows any `1.x.y`, since no minor version
/// was given to hold fixed.
fn tilde_range(bound: &str) -> Option<VersionRange> {
    let components = parse_components(bound)?;
    let lower = lower_from_components(&components);
    let upper = match components.len() {
        0 => return Some(VersionRange { lower, upper: None }),
        1 => Version {
            major: components[0] + 1,
            minor: 0,
            patch: 0,
            build: 0,
        },
        _ => Version {
            major: components[0],
            minor: components[1] + 1,
            patch: 0,
            build: 0,
        },
    };
    Some(VersionRange {
        lower,
        upper: Some(upper),
    })
}

/// Expands an exact (`=`) bound.
///
/// Simplification: like the registry's own compressed range keys, a partial
/// bound such as `"=1.2"` matches every `1.2.x` release rather than only a
/// literal version `1.2.0`. Exact compat pins in the wild are almost always
/// given at full `major.minor.patch` precision, where this distinction does
/// not matter.
fn exact_range(bound: &str) -> Option<VersionRange> {
    let components = parse_components(bound)?;
    Some(VersionRange {
        lower: lower_from_components(&components),
        upper: upper_exclusive_from_components(&components),
    })
}

/// Expands a strict less-than bound such as `"< 0.0.1"`, which real
/// `Project.toml` files use for Julia standard library pseudo-dependencies
/// (for example `Libdl = "< 0.0.1, 1"`). Unlike the caret, tilde, and exact
/// bounds above, this is a literal upper bound on the exact value given,
/// not "one step past the last component".
fn less_than_range(bound: &str) -> Option<VersionRange> {
    let components = parse_components(bound)?;
    Some(VersionRange {
        lower: Version::default(),
        upper: Some(lower_from_components(&components)),
    })
}

/// Expands a hyphen range such as `"1.6.0-1"`, meaning from the first bound
/// up to (but excluding) one step past the last component of the second
/// bound. Reuses the same component expansion as [`exact_range`], since a
/// hyphen range is exactly a lower bound and an upper bound expressed the
/// same way an exact bound expresses each side.
fn hyphen_range(token: &str) -> Option<VersionRange> {
    let (lower_bound, upper_bound) = token.split_once('-')?;
    let lower_components = parse_components(lower_bound.trim())?;
    let upper_components = parse_components(upper_bound.trim())?;
    Some(VersionRange {
        lower: lower_from_components(&lower_components),
        upper: upper_exclusive_from_components(&upper_components),
    })
}

/// Parses one comma-separated piece of a Julia compat specifier.
///
/// Besides the caret (default), tilde, and exact prefixes, a bare token can
/// also be a strict less-than bound (`"< 0.0.1"`) or an explicit hyphen
/// range (`"1.6.0-1"`). An unrecognised token (one whose numeric components
/// do not parse, for example a `>=` bound, which no real JLL `Project.toml`
/// has been observed to use) is treated as unbounded rather than a hard
/// failure, so an odd bound never blocks generation. This is a documented
/// caveat: such a token's constraint is silently dropped instead of
/// enforced.
fn parse_compat_token(token: &str) -> VersionRange {
    let token = token.trim();
    if token.is_empty() || token == "*" {
        return VersionRange::unbounded();
    }

    let parsed = if let Some(rest) = token.strip_prefix('<') {
        less_than_range(rest.trim())
    } else if let Some(rest) = token.strip_prefix('=') {
        exact_range(rest.trim())
    } else if let Some(rest) = token.strip_prefix('~') {
        tilde_range(rest.trim())
    } else if let Some(rest) = token.strip_prefix('^') {
        caret_range(rest.trim())
    } else if token.contains('-') {
        hyphen_range(token)
    } else {
        caret_range(token)
    };

    parsed.unwrap_or_else(VersionRange::unbounded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_version() {
        let version = Version::parse("7.12.1+0").unwrap();
        assert_eq!(
            version,
            Version {
                major: 7,
                minor: 12,
                patch: 1,
                build: 0
            }
        );
    }

    #[test]
    fn parses_a_version_with_missing_components() {
        assert_eq!(
            Version::parse("1.11").unwrap(),
            Version {
                major: 1,
                minor: 11,
                patch: 0,
                build: 0
            }
        );
    }

    #[test]
    fn orders_by_build_within_the_same_release() {
        let earlier = Version::parse("7.8.3+1").unwrap();
        let later = Version::parse("7.8.3+2").unwrap();
        assert!(earlier < later);
    }

    #[test]
    fn orders_patch_ahead_of_build() {
        let lower_patch_higher_build = Version::parse("7.8.3+99").unwrap();
        let higher_patch = Version::parse("7.8.4+0").unwrap();
        assert!(lower_patch_higher_build < higher_patch);
    }

    #[test]
    fn rejects_a_non_numeric_component() {
        assert!(Version::parse("1.x.0").is_err());
    }

    #[test]
    fn caret_bound_leftmost_nonzero_rule() {
        let specifier = CompatSpecifier::parse("1.2.3");
        assert!(specifier.contains(Version::parse("1.2.3").unwrap()));
        assert!(specifier.contains(Version::parse("1.9.9").unwrap()));
        assert!(!specifier.contains(Version::parse("2.0.0").unwrap()));

        let specifier = CompatSpecifier::parse("^0.2.3");
        assert!(specifier.contains(Version::parse("0.2.9").unwrap()));
        assert!(!specifier.contains(Version::parse("0.3.0").unwrap()));

        let specifier = CompatSpecifier::parse("^0.0.3");
        assert!(specifier.contains(Version::parse("0.0.3").unwrap()));
        assert!(!specifier.contains(Version::parse("0.0.4").unwrap()));
    }

    #[test]
    fn compat_floor_accepts_a_build_bump() {
        // This is the case that matters for JLL dependency floors: a
        // "5.8.0" bound must accept "5.8.0+3", not just "5.8.0+0".
        let specifier = CompatSpecifier::parse("5.8.0");
        assert!(specifier.contains(Version::parse("5.8.0+3").unwrap()));
        assert!(specifier.contains(Version::parse("5.9.0+0").unwrap()));
        assert!(!specifier.contains(Version::parse("6.0.0+0").unwrap()));
    }

    #[test]
    fn tilde_bound() {
        let specifier = CompatSpecifier::parse("~1.2.3");
        assert!(specifier.contains(Version::parse("1.2.9").unwrap()));
        assert!(!specifier.contains(Version::parse("1.3.0").unwrap()));

        let specifier = CompatSpecifier::parse("~1");
        assert!(specifier.contains(Version::parse("1.9.9").unwrap()));
        assert!(!specifier.contains(Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn exact_bound_matches_the_whole_partial_family() {
        let specifier = CompatSpecifier::parse("=1.2");
        assert!(specifier.contains(Version::parse("1.2.0").unwrap()));
        assert!(specifier.contains(Version::parse("1.2.9").unwrap()));
        assert!(!specifier.contains(Version::parse("1.3.0").unwrap()));
    }

    #[test]
    fn strict_less_than_bound() {
        let specifier = CompatSpecifier::parse("< 0.0.1");
        assert!(specifier.contains(Version::parse("0.0.0").unwrap()));
        assert!(!specifier.contains(Version::parse("0.0.1").unwrap()));
    }

    #[test]
    fn hyphen_bound() {
        let specifier = CompatSpecifier::parse("1.6.0-1");
        assert!(specifier.contains(Version::parse("1.6.0").unwrap()));
        assert!(specifier.contains(Version::parse("1.9.9").unwrap()));
        assert!(!specifier.contains(Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn comma_union() {
        // This is the real bound libdl-style stdlib deps use.
        let specifier = CompatSpecifier::parse("< 0.0.1, 1");
        assert!(!specifier.contains(Version::parse("0.0.1").unwrap()));
        assert!(specifier.contains(Version::parse("1.5.0").unwrap()));
        assert!(!specifier.contains(Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn wildcard_and_empty_are_unbounded() {
        let specifier = CompatSpecifier::parse("*");
        assert!(specifier.contains(Version::parse("999.0.0").unwrap()));
    }

    #[test]
    fn an_unrecognised_token_is_treated_as_unbounded() {
        let specifier = CompatSpecifier::parse("not-a-version");
        assert!(specifier.contains(Version::parse("0.0.1").unwrap()));
    }
}
