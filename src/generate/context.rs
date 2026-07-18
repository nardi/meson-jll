//! The small value types askama templates render from.
//!
//! Every field a template needs is worked out here in plain Rust first, so
//! the templates themselves stay simple substitution and iteration, with no
//! conditions or string building left for the template language to do. See
//! [`crate::generate`] for how these are assembled and rendered.

use askama::Template;

/// Turns a JLL package name into the Meson variable name its dependency
/// object is bound to, for example `SuiteSparse` becomes `suitesparse_dep`.
pub fn dependency_variable(name: &str) -> String {
    format!("{}_dep", name.to_lowercase())
}

/// Derives the plain library name Meson's `cc.find_library()` expects (for
/// example `amd`) from a JLL library path such as `lib/libamd.so` or
/// `bin/libamd.dll`.
pub fn link_name_from_path(path: &str) -> String {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let before_first_dot = file_name.split('.').next().unwrap_or(file_name);
    before_first_dot
        .strip_prefix("lib")
        .unwrap_or(before_first_dot)
        .to_string()
}

/// Renders `SuiteSparse.wrap`, the overlay-only selector wrap.
#[derive(Template)]
#[template(path = "selector_wrap.jinja", escape = "none")]
pub struct SelectorWrapContext<'a> {
    /// The bare package name, also used as the public dependency name.
    pub name: &'a str,
    /// The JLL release version this wrap was generated from. Recorded in a
    /// marker comment so that later `status` and `update` runs can see what
    /// is installed without re-fetching anything. See
    /// [`crate::status`](crate::status).
    pub version: &'a str,
}

/// Renders `SuiteSparse-<triplet>.wrap`, a normal binary wrap for one
/// platform's tarball.
#[derive(Template)]
#[template(path = "binary_wrap.jinja", escape = "none")]
pub struct BinaryWrapContext<'a> {
    pub directory: &'a str,
    pub source_url: &'a str,
    pub source_filename: &'a str,
    pub source_hash: &'a str,
}

/// Renders `meson.options`, exposing ABI variant choices as build options.
/// Only written when a package actually splits at least one platform by
/// `cxxstring_abi` or `libgfortran_version`.
#[derive(Template)]
#[template(path = "options.jinja", escape = "none")]
pub struct OptionsContext {
    pub has_cxxstring_abi: bool,
    /// A ready-to-interpolate Meson list literal body, for example
    /// `'cxx03', 'cxx11'`.
    pub cxxstring_choices: String,
    pub cxxstring_default: String,
    pub has_libgfortran: bool,
    pub libgfortran_choices: String,
    pub libgfortran_default: String,
}

/// One platform's entry in the selector's if-elif chain.
pub struct PlatformSelector {
    pub identifier: String,
    /// The full Meson boolean expression that matches this platform,
    /// precomputed so the template only interpolates a string.
    pub condition: String,
}

/// Renders the selector overlay's `meson.build`, which maps the host
/// machine to a triplet and delegates to that triplet's subproject.
#[derive(Template)]
#[template(path = "selector_overlay.jinja", escape = "none")]
pub struct SelectorOverlayContext<'a> {
    pub name: &'a str,
    pub dependency_variable: String,
    pub needs_libc_probe: bool,
    pub has_abi_options: bool,
    pub platforms: Vec<PlatformSelector>,
}

/// One library product as it appears in a per-triplet overlay.
pub struct LibraryProductView {
    pub variable: String,
    pub link_name: String,
}

/// Renders a per-triplet overlay's `meson.build`, which turns the extracted
/// binary tree into a `declare_dependency()`.
#[derive(Template)]
#[template(path = "triplet_overlay.jinja", escape = "none")]
pub struct TripletOverlayContext<'a> {
    /// The per-triplet subproject name, for example
    /// `SuiteSparse-x86_64-linux-gnu`.
    pub name: &'a str,
    pub dependency_variable: String,
    pub library_products: Vec<LibraryProductView>,
    /// The bare names of the other JLL packages this platform links against.
    pub jll_dependencies: Vec<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_variable_is_lowercased() {
        assert_eq!(dependency_variable("SuiteSparse"), "suitesparse_dep");
    }

    #[test]
    fn link_name_strips_directory_lib_prefix_and_extension() {
        assert_eq!(link_name_from_path("lib/libamd.so"), "amd");
        assert_eq!(link_name_from_path("lib/libcholmod.so"), "cholmod");
        assert_eq!(link_name_from_path("bin/libamd.dll"), "amd");
    }

    #[test]
    fn link_name_handles_a_versioned_soname() {
        assert_eq!(link_name_from_path("lib/libamd.so.3"), "amd");
    }
}
