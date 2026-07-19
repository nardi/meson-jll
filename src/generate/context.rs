//! The small value types askama templates render from.
//!
//! Every field a template needs is worked out here in plain Rust first, so
//! the templates themselves stay simple substitution and iteration, with no
//! conditions or string building left for the template language to do. See
//! [`crate::generate`] for how these are assembled and rendered.

use askama::Template;

/// Turns a JLL package name into the Meson variable name its dependency
/// object is bound to, for example `ExampleThing` becomes
/// `examplething_dep`.
pub fn dependency_variable(name: &str) -> String {
    format!("{}_dep", name.to_lowercase())
}

/// Normalises a JLL library path to forward slashes, for example turning
/// `bin\\libexample.dll` into `bin/libexample.dll`.
///
/// The wrapper script parser does not decode Julia string escapes (see
/// `crate::jll::wrappers`), so a Windows path arrives with the doubled
/// backslash still literally doubled: two backslash characters, standing
/// for the one real separator Julia's own string would contain. Replacing
/// that exact two-character sequence first, before falling back to
/// replacing any single stray backslash, avoids turning it into two
/// forward slashes instead of one.
pub fn normalize_path(path: &str) -> String {
    path.replace("\\\\", "/").replace('\\', "/")
}

/// Derives the plain library name Meson's `cc.find_library()` expects (for
/// example `example`) from a JLL library path such as `lib/libexample.so`
/// or `bin\\libexample.dll`.
///
/// The path's directory is deliberately not used for anything: it is where
/// Julia's `@init_library_product` `dlopen`s the library from at runtime,
/// which on Windows is the DLL itself under `bin/`, not the import library
/// `find_library` actually needs to link against. Julia's own JLLWrappers
/// build always places that import library under `lib/` instead (as
/// `lib<name>.dll.a` on a MinGW target, matching `lib<name>.so` /
/// `lib<name>.dylib` on the platforms where the runtime and link-time
/// files are one and the same), so the overlay always searches `lib/`
/// regardless of platform, and only the name is taken from this path.
pub fn link_name_from_path(path: &str) -> String {
    let normalized = normalize_path(path);
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let before_first_dot = file_name.split('.').next().unwrap_or(file_name);
    before_first_dot
        .strip_prefix("lib")
        .unwrap_or(before_first_dot)
        .to_string()
}

/// Renders the overlay-only selector wrap, for example `ExampleThing.wrap`.
#[derive(Template)]
#[template(path = "selector_wrap.jinja", escape = "none")]
pub struct SelectorWrapContext<'a> {
    /// The bare package name, also used as the public dependency name.
    pub name: &'a str,
    /// The JLL release version this wrap was generated from. Recorded in a
    /// marker comment so that later `status` and `update` runs can see what
    /// is installed without re-fetching anything. See
    /// [`crate::status`].
    pub version: &'a str,
    /// The name of the shared placeholder archive this wrap points its
    /// mandatory `source_filename` at. See
    /// [`crate::generate::EMPTY_TAR_FILENAME`].
    pub empty_tar_filename: &'a str,
}

/// Renders `ExampleThing-<triplet>.wrap`, a normal binary wrap for one
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
    /// Whether at least one platform splits by C++ standard library ABI,
    /// matching `OptionsContext::has_cxxstring_abi`: `meson.options` only
    /// declares `cxxstring_abi` when this is set, so `get_option` for it
    /// must be guarded by the same condition, not lumped in with
    /// `has_libgfortran`.
    pub has_cxxstring_abi: bool,
    /// Whether at least one platform splits by Fortran runtime version, the
    /// `libgfortran_version` counterpart to `has_cxxstring_abi` above.
    pub has_libgfortran: bool,
    pub platforms: Vec<PlatformSelector>,
}

/// One library product as it appears in a per-triplet overlay.
pub struct LibraryProductView {
    pub variable: String,
    pub link_name: String,
    /// The path Julia's own wrapper script declared for this library,
    /// relative to the extracted tarball, normalised to forward slashes
    /// (see [`link_name_from_path`]). Kept as-is, versioned soname and
    /// all, rather than reconstructed from `link_name`, so the file
    /// `install_data()` installs is always the exact one that actually
    /// exists on disk.
    pub path: String,
}

/// Renders a per-triplet overlay's `meson.build`, which turns the extracted
/// binary tree into a `declare_dependency()`.
#[derive(Template)]
#[template(path = "triplet_overlay.jinja", escape = "none")]
pub struct TripletOverlayContext<'a> {
    /// The per-triplet subproject name, for example
    /// `ExampleThing-x86_64-linux-gnu`.
    pub name: &'a str,
    pub dependency_variable: String,
    pub library_products: Vec<LibraryProductView>,
    /// The bare names of the other JLL packages this platform links against.
    pub jll_dependencies: Vec<&'a str>,
    /// The package's own bare name, lowercased. A number of JLLs (SuiteSparse
    /// and HiGHS both do this) install their headers into `include/<this>/`
    /// instead of flat under `include/`, to avoid collisions between
    /// generically named headers (`config.h`) from different JLLs used
    /// together. Checked for and added as an extra include directory
    /// alongside plain `include` when it exists, since nothing in a JLL's
    /// own metadata says whether it follows this convention.
    pub namespaced_include_dir: String,
    /// Whether this platform is Windows. Julia's Windows binaries are
    /// MinGW-w64 built, so their import libraries (`.dll.a`) cannot be
    /// read by MSVC's linker. When set, the template regenerates an
    /// MSVC-compatible `.lib` straight from each DLL's own export table
    /// (see [`Self::msvc_machine`] and the sibling `dll_to_lib.py` this
    /// overlay writes next to itself) whenever the active compiler turns
    /// out to be MSVC and no native `.lib` already exists. Irrelevant, and
    /// unused by the template, on every other platform.
    pub is_windows: bool,
    /// The value MSVC's `lib.exe /machine:` flag expects on this
    /// architecture (see [`crate::jll::triplet::Arch::msvc_machine`]).
    /// Only meaningful when `is_windows` is set.
    pub msvc_machine: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_variable_is_lowercased() {
        assert_eq!(dependency_variable("ExampleThing"), "examplething_dep");
    }

    #[test]
    fn link_name_strips_directory_lib_prefix_and_extension() {
        assert_eq!(link_name_from_path("lib/libexample.so"), "example");
        assert_eq!(link_name_from_path("lib/libother.so"), "other");
        assert_eq!(link_name_from_path("bin/libexample.dll"), "example");
    }

    #[test]
    fn link_name_handles_a_windows_backslash_path() {
        // Julia's wrapper source doubles the backslash, so the parser (which
        // does not decode escapes) reads this as two literal backslash
        // characters, not one: "bin\\\\libexample.dll" here is Rust's own
        // escaping of that two-character sequence.
        assert_eq!(link_name_from_path("bin\\\\libexample.dll"), "example");
    }

    #[test]
    fn normalize_path_collapses_a_doubled_backslash_to_one_slash() {
        assert_eq!(
            normalize_path("bin\\\\libexample.dll"),
            "bin/libexample.dll"
        );
    }

    #[test]
    fn normalize_path_leaves_a_forward_slash_path_unchanged() {
        assert_eq!(normalize_path("lib/libexample.so"), "lib/libexample.so");
    }

    #[test]
    fn link_name_handles_a_versioned_soname() {
        assert_eq!(link_name_from_path("lib/libexample.so.3"), "example");
    }

    /// Regression test: the selector overlay used to guard both
    /// `get_option` calls on one combined flag, so a package split only by
    /// `libgfortran_version` (no `cxxstring_abi` variants at all) still
    /// generated a `get_option('cxxstring_abi')` call, one `meson.options`
    /// never declares that option for, failing at Meson configure time.
    #[test]
    fn only_declares_get_option_for_axes_the_package_actually_splits_by() {
        let context = SelectorOverlayContext {
            name: "ExampleThing",
            dependency_variable: "examplething_dep".to_string(),
            needs_libc_probe: false,
            has_cxxstring_abi: false,
            has_libgfortran: true,
            platforms: Vec::new(),
        };
        let rendered = context.render().unwrap();
        assert!(rendered.contains("get_option('libgfortran_version')"));
        assert!(!rendered.contains("get_option('cxxstring_abi')"));
    }
}
