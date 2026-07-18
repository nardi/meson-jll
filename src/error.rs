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
        source: toml::de::Error,
    },

    /// `Artifacts.toml` could not be parsed as TOML.
    #[error("could not parse Artifacts.toml: {source}")]
    ParseArtifactsToml {
        #[source]
        source: toml::de::Error,
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
}

/// A [`Result`](std::result::Result) alias using this crate's [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;
