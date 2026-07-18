//! Progress events emitted while installing, updating, or regenerating JLL
//! wraps.
//!
//! Resolving a package's version and writing its wrap set each make one or
//! more network round trips per package (see `crate::resolve` and
//! `crate::install`), so a JLL with several of its own JLL dependencies, or
//! a project with several installed JLLs, can take a visible amount of
//! time. This event stream lets a caller drive a spinner and print timed
//! per-phase summaries, the way `meson-jll`'s own binary does, without the
//! library itself printing anything: printing is the binary's job (see the
//! crate root documentation).

/// One step of progress while installing, updating, or regenerating a JLL
/// wrap set.
pub enum Progress<'a> {
    /// About to resolve `name`'s version: a network round trip for its
    /// tags, and (the first time this version is seen) another for its
    /// chosen version's `Project.toml`.
    Resolving(&'a str),
    /// Resolving finished; `count` packages were resolved in total.
    Resolved { count: usize },
    /// About to fetch `name`'s full metadata and write its wrap set.
    Writing(&'a str),
}
