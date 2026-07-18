//! The single error type shared by every part of the library.
//!
//! Collecting every failure mode into one enum lets callers match on the
//! cause without reaching for a boxed trait object, in line with this
//! project's preference for static dispatch over dynamic dispatch.

use std::path::PathBuf;

/// Every way a `meson-jll` operation can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A network request could not be completed at all, for example because
    /// the host could not be reached or it returned an error status.
    #[error("could not fetch {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    /// A request succeeded, but its response body could not be read as text.
    #[error("could not read the response body from {url}: {source}")]
    ReadResponseBody {
        url: String,
        #[source]
        source: std::io::Error,
    },

    /// A local file, read as a fallback source for `--url`, could not be read.
    #[error("could not read {path}: {source}")]
    ReadLocalFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A response from the GitHub API could not be parsed as the JSON shape
    /// this tool expects.
    #[error("could not parse the response from {url} as JSON: {source}")]
    ParseJson {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    /// `Project.toml` could not be parsed as TOML.
    #[error("could not parse Project.toml: {source}")]
    ParseProjectToml {
        #[source]
        source: Box<toml::de::Error>,
    },

    /// `Artifacts.toml` could not be parsed as TOML.
    #[error("could not parse Artifacts.toml: {source}")]
    ParseArtifactsToml {
        #[source]
        source: Box<toml::de::Error>,
    },

    /// `Artifacts.toml` was parsed, but it had no entry matching this
    /// package's name, so there is nothing to generate a wrap from.
    #[error("Artifacts.toml has no platform entries for package {name}")]
    NoPlatforms { name: String },

    /// The given name does not look like a JLL package name.
    #[error(
        "{name} does not look like a JLL package name (expected a bare name or one ending in _jll)"
    )]
    NotAJllName { name: String },

    /// `install` (without `--force`) refused to overwrite a file that
    /// already exists.
    #[error("{path} already exists, pass --force to overwrite it")]
    AlreadyExists { path: PathBuf },

    /// A generated file could not be written to disk.
    #[error("could not write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The directory a generated file belongs in could not be created.
    #[error("could not create directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Rendering a template into a generated file failed.
    #[error("could not render {template}: {source}")]
    Render {
        template: &'static str,
        #[source]
        source: askama::Error,
    },

    /// A version string (a JLL release version, or one component of a
    /// compat bound) was not a plain dotted sequence of non-negative
    /// integers, with an optional `+build` suffix.
    #[error("{text} is not a valid version")]
    ParseVersion { text: String },

    /// The lockfile could not be parsed as TOML.
    #[error("could not parse the lockfile at {path}: {source}")]
    ParseLockFile {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },

    /// The lockfile model could not be serialized back to TOML. This should
    /// never actually happen for the plain data the lockfile holds, but the
    /// underlying library call is fallible.
    #[error("could not serialize the lockfile: {source}")]
    SerializeLockFile {
        #[source]
        source: Box<toml::ser::Error>,
    },

    /// The lockfile's `version` field is not one this build of `meson-jll`
    /// understands. See `crate::lockfile` for what the field means.
    #[error(
        "the lockfile at {path} is format version {found}, but this build of meson-jll only understands version {supported}. Upgrade meson-jll to read it."
    )]
    UnsupportedLockFileVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    /// A name passed to the resolver has no published versions at all, so
    /// it cannot be a real JLL package.
    #[error("{name} is not a known JLL package")]
    UnknownJllPackage { name: String },

    /// A name resolves to a real JLL repository only once case is ignored.
    /// JLL names are case-sensitive everywhere except this one lookup, so
    /// the mismatch is reported rather than silently corrected.
    #[error("{given} is not a published JLL package name, did you mean {suggested}?")]
    WrongCase { given: String, suggested: String },

    /// The system `git` binary could not even be started, for example
    /// because it is not installed or not on `PATH`.
    #[error("could not run git {}: {source}", args.join(" "))]
    RunGit {
        args: Vec<String>,
        #[source]
        source: std::io::Error,
    },

    /// A git command ran but exited with a failure.
    #[error("git {} failed: {stderr}", args.join(" "))]
    GitFailed { args: Vec<String>, stderr: String },

    /// A `pins` entry named a version that is not actually published for
    /// that package.
    #[error("{name} has no published version {pin}")]
    UnknownPin { name: String, pin: String },

    /// No available version of a package satisfies every `[compat]` bound
    /// accumulated against it during resolution.
    #[error("no version of {name} satisfies the compat ranges required of it")]
    NoSatisfyingVersion { name: String },

    /// The fixed-point resolver did not settle within its iteration budget.
    /// In practice this means the required JLLs have compat ranges that
    /// keep forcing each other to different versions pass after pass.
    #[error(
        "dependency resolution did not converge after {max_iterations} passes, the required JLLs may have conflicting compat ranges"
    )]
    ResolutionDidNotConverge { max_iterations: u32 },
}

/// A [`Result`](std::result::Result) alias using this crate's [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;
