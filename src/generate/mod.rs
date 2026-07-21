//! Rendering and writing a JLL package's generated wrap set.
//!
//! [`write_wrap_set`] takes one resolved [`JllPackage`] and writes the
//! selector wrap, the selector overlay, the per-triplet binary wraps, and
//! the per-triplet overlays into a project's `subprojects/` directory. See
//! the crate documentation for what each of these files is for. This
//! function does not recurse into JLL dependencies; the caller walks the
//! dependency graph and calls it once per package.

pub mod context;

use std::fs;
use std::path::Path;

use askama::Template;

use crate::error::{Error, Result};
use crate::jll::triplet::Os;
use crate::jll::{JllPackage, ResolvedPlatform};
use context::{
    dependency_variable, link_name_from_path, normalize_path, BinaryWrapContext,
    LibraryProductView, OptionsContext, PlatformSelector, SelectorOverlayContext,
    SelectorWrapContext, TripletOverlayContext,
};

/// The name of the placeholder archive every selector wrap's
/// `source_filename` points at.
///
/// Meson requires a `[wrap-file]` section to declare a source, even one
/// that only exists to load a `patch_directory` overlay and never actually
/// builds anything from its own source tree. Rather than downloading a real
/// (and pointless) archive for every package, every selector wrap shares one
/// local, empty tar file, placed directly under `subprojects/packagefiles/`
/// the same way any other locally supplied wrap source would be.
pub const EMPTY_TAR_FILENAME: &str = "_meson-jll-empty.tar";

/// The bytes of a valid, empty tar archive: two 512-byte all-zero records,
/// which is the standard end-of-archive marker and nothing else.
const EMPTY_TAR_BYTES: [u8; 1024] = [0u8; 1024];

/// The name [`DLL_TO_LIB_SCRIPT`] is written under, directly in
/// `packagefiles/`, shared by every Windows triplet overlay the same way
/// [`EMPTY_TAR_FILENAME`] is: written once, referenced by every consumer
/// via a relative path up from its own overlay directory, rather than
/// duplicated into each one.
const DLL_TO_LIB_FILENAME: &str = "dll_to_lib.py";

/// A small, generic Python script that regenerates an MSVC-compatible
/// `.lib` from a DLL's own export table, using `dumpbin` and `lib.exe`
/// (see the "MSVC bridging" section of `triplet_overlay.jinja`, the only
/// place this is invoked from, at Meson build time). Both tools are part
/// of the same MSVC installation as the compiler Meson already activated
/// to run the build, so nothing beyond that is required.
///
/// This exists because Julia's Windows JLL binaries are built with
/// MinGW-w64 GCC, whose `.dll.a` import libraries are a GNU `ar` archive
/// format MSVC's linker cannot read at all, not merely a different naming
/// convention. Regenerating an equivalent import library straight from the
/// DLL's own export table sidesteps that instead of requiring a MinGW
/// toolchain just to consume a prebuilt binary.
const DLL_TO_LIB_SCRIPT: &str = r#"#!/usr/bin/env python3
"""Regenerates an MSVC-compatible import library from a DLL's own export
table, using dumpbin and lib.exe. See the meson-jll comment that generated
this file for why this exists.

Usage: dll_to_lib.py <dll-path> <output-lib-path> <machine>
"""
import os
import re
import subprocess
import sys
import tempfile


def main():
    dll_path, output_path, machine = sys.argv[1:4]

    exports = subprocess.run(
        ["dumpbin", "/exports", dll_path],
        capture_output=True,
        text=True,
        check=True,
    ).stdout

    # Each exported symbol's line looks like:
    #   1    0 00001080 amd_control
    # (ordinal, hint, RVA, name), the name being the last column.
    names = []
    for line in exports.splitlines():
        match = re.match(r"^\s*\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\S+)", line)
        if match:
            names.append(match.group(1))

    # lib.exe embeds a LIBRARY statement's name as the DLL to load at
    # runtime. Without one, it falls back to the .def file's own base
    # name, which is this script's randomly named temp file, not the DLL
    # actually being wrapped, silently producing an import library that
    # points at a DLL that does not exist.
    dll_name = os.path.basename(dll_path)

    definition_fd, definition_path = tempfile.mkstemp(suffix=".def")
    try:
        with os.fdopen(definition_fd, "w") as definition_file:
            definition_file.write(f'LIBRARY "{dll_name}"\n')
            definition_file.write("EXPORTS\n")
            for name in names:
                definition_file.write(f"{name}\n")

        subprocess.run(
            [
                "lib",
                f"/def:{definition_path}",
                f"/out:{output_path}",
                f"/machine:{machine}",
            ],
            check=True,
        )
    finally:
        os.unlink(definition_path)


if __name__ == "__main__":
    main()
"#;

/// The name [`STRIP_LIBS_SCRIPT`] is written under, directly in
/// `packagefiles/`, shared the same way [`DLL_TO_LIB_FILENAME`] is.
pub const STRIP_LIBS_FILENAME: &str = "strip_libs.py";

/// A small, generic Python script that strips debug info and symbol
/// tables from every file a triplet overlay just installed, using
/// whichever strip tool Meson found (see the "strip" section of
/// `triplet_overlay.jinja`, the only place this is invoked from, at
/// Meson install time, and only when `-Dstrip` is set).
///
/// This exists because JLL binaries ship unstripped: a bundled
/// `libstdc++` commonly carries ten times its stripped size in symbols.
/// Meson's own `-Dstrip` only strips targets Meson itself compiled, never
/// a file `install_subdir` copied in verbatim, so without this a project
/// built with `-Dstrip=true` would still ship every JLL dependency fully
/// unstripped.
const STRIP_LIBS_SCRIPT: &str = r#"#!/usr/bin/env python3
"""Strips every file Meson just installed into libdir, using whichever
strip tool Meson found. See the meson-jll comment that generated this
file for why this exists.

Usage: strip_libs.py <strip-tool> <libdir-relative-path>
"""
import os
import subprocess
import sys


def main():
    strip_tool, libdir = sys.argv[1:3]

    # MESON_INSTALL_DESTDIR_PREFIX is only known once `meson install`
    # actually runs (it can be overridden with --destdir or --prefix at
    # install time), so the libdir this script strips is resolved here,
    # never baked into the command Meson was configured with.
    destdir_prefix = os.environ["MESON_INSTALL_DESTDIR_PREFIX"]
    target_dir = os.path.join(destdir_prefix, libdir)
    if not os.path.isdir(target_dir):
        return

    for name in os.listdir(target_dir):
        path = os.path.join(target_dir, name)
        if os.path.isfile(path):
            # check=False: a stray non-object file left in libdir should
            # not fail the whole install, only skip stripping.
            subprocess.run([strip_tool, "--strip-all", path], check=False)


if __name__ == "__main__":
    main()
"#;

/// Writes the full wrap set for `package` into `subprojects_dir` (normally
/// a project's `subprojects/` directory).
///
/// Existing files are left untouched, and this returns
/// [`Error::AlreadyExists`] for the first one found, unless `force` is set.
pub fn write_wrap_set(package: &JllPackage, subprojects_dir: &Path, force: bool) -> Result<()> {
    let dependency_variable_name = dependency_variable(&package.name);

    write_empty_tar(subprojects_dir)?;
    write_strip_libs_script(subprojects_dir)?;
    write_selector_wrap(package, subprojects_dir, force)?;
    write_selector_overlay(package, &dependency_variable_name, subprojects_dir, force)?;
    write_options(package, subprojects_dir, force)?;

    if package
        .platforms
        .iter()
        .any(|resolved| resolved.platform.triplet.os == Os::Windows)
    {
        write_dll_to_lib_script(subprojects_dir)?;
    }

    for resolved in &package.platforms {
        write_binary_wrap(package, resolved, subprojects_dir, force)?;
        write_triplet_overlay(
            package,
            resolved,
            &dependency_variable_name,
            subprojects_dir,
            force,
        )?;
    }

    Ok(())
}

/// Writes the shared `strip_libs.py`, if it is not already there. Every
/// platform's triplet overlay references it, not only Windows's, so
/// (unlike [`write_dll_to_lib_script`]) this always runs.
fn write_strip_libs_script(subprojects_dir: &Path) -> Result<()> {
    let path = subprojects_dir
        .join("packagefiles")
        .join(STRIP_LIBS_FILENAME);
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&path, STRIP_LIBS_SCRIPT).map_err(|source| Error::WriteFile { path, source })
}

/// Writes the shared empty tar archive, if it is not already there.
///
/// Its bytes never change, so unlike the rest of the generated files this
/// is always safe to leave in place rather than gating it on `force`.
fn write_empty_tar(subprojects_dir: &Path) -> Result<()> {
    let path = subprojects_dir
        .join("packagefiles")
        .join(EMPTY_TAR_FILENAME);
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&path, EMPTY_TAR_BYTES).map_err(|source| Error::WriteFile { path, source })
}

/// Writes the shared `dll_to_lib.py`, if it is not already there. See
/// [`DLL_TO_LIB_FILENAME`] for why this lives directly under
/// `packagefiles/` instead of inside each triplet overlay.
fn write_dll_to_lib_script(subprojects_dir: &Path) -> Result<()> {
    let path = subprojects_dir
        .join("packagefiles")
        .join(DLL_TO_LIB_FILENAME);
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&path, DLL_TO_LIB_SCRIPT).map_err(|source| Error::WriteFile { path, source })
}

fn write_selector_wrap(package: &JllPackage, subprojects_dir: &Path, force: bool) -> Result<()> {
    let context = SelectorWrapContext {
        name: &package.name,
        version: &package.version,
        empty_tar_filename: EMPTY_TAR_FILENAME,
    };
    let rendered = render(&context, "selector_wrap.jinja")?;
    let path = subprojects_dir.join(format!("{}.wrap", package.name));
    write_generated_file(&path, &rendered, force)
}

fn write_binary_wrap(
    package: &JllPackage,
    resolved: &ResolvedPlatform,
    subprojects_dir: &Path,
    force: bool,
) -> Result<()> {
    let identifier = resolved.platform.triplet.identifier();
    let directory = format!("{}-{identifier}", package.name);
    let source_filename = resolved
        .platform
        .source_url
        .rsplit('/')
        .next()
        .unwrap_or(&resolved.platform.source_url);

    let context = BinaryWrapContext {
        directory: &directory,
        source_url: &resolved.platform.source_url,
        source_filename,
        source_hash: &resolved.platform.source_hash,
    };
    let rendered = render(&context, "binary_wrap.jinja")?;
    let path = subprojects_dir.join(format!("{directory}.wrap"));
    write_generated_file(&path, &rendered, force)
}

fn write_triplet_overlay(
    package: &JllPackage,
    resolved: &ResolvedPlatform,
    dependency_variable_name: &str,
    subprojects_dir: &Path,
    force: bool,
) -> Result<()> {
    let identifier = resolved.platform.triplet.identifier();
    let project_name = format!("{}-{identifier}", package.name);

    let library_products = resolved
        .library_products
        .iter()
        .map(|product| LibraryProductView {
            variable: product.variable.clone(),
            link_name: link_name_from_path(&product.path),
            path: normalize_path(&product.path),
        })
        .collect();

    let is_windows = resolved.platform.triplet.os == Os::Windows;
    let overlay_dir = subprojects_dir.join("packagefiles").join(&project_name);

    let mut jll_dependencies: Vec<&str> = package.dependencies.iter().map(String::as_str).collect();
    if is_windows
        && package.name != crate::install::WINDOWS_RUNTIME_SHIM_PACKAGE
        && !jll_dependencies.contains(&crate::install::WINDOWS_RUNTIME_SHIM_PACKAGE)
    {
        // Referenced here purely so Meson actually configures, builds, and
        // installs this subproject at all: nothing else in the generated
        // wrap set otherwise calls dependency('CompilerSupportLibraries'),
        // and a subproject nothing references never runs, regardless of
        // whether meson-jll already wrote its wrap files to disk. See
        // `crate::install::WINDOWS_RUNTIME_SHIM_PACKAGE` for why every
        // Windows platform needs it.
        jll_dependencies.push(crate::install::WINDOWS_RUNTIME_SHIM_PACKAGE);
    }

    let context = TripletOverlayContext {
        name: &project_name,
        version: &package.version,
        dependency_variable: dependency_variable_name.to_string(),
        library_products,
        jll_dependencies,
        namespaced_include_dir: package.name.to_lowercase(),
        is_windows,
        msvc_machine: resolved
            .platform
            .triplet
            .arch
            .msvc_machine()
            .unwrap_or_default(),
    };
    let rendered = render(&context, "triplet_overlay.jinja")?;
    let path = overlay_dir.join("meson.build");
    write_generated_file(&path, &rendered, force)
}

fn write_selector_overlay(
    package: &JllPackage,
    dependency_variable_name: &str,
    subprojects_dir: &Path,
    force: bool,
) -> Result<()> {
    let needs_libc_probe = package
        .platforms
        .iter()
        .any(|resolved| resolved.platform.triplet.libc.is_some());
    // Computed independently, matching `write_options` below exactly: a
    // JLL can split by only one of these two axes, and `meson.options`
    // only declares the option for the axis that actually applies, so the
    // template must guard each `get_option` call on its own flag rather
    // than a single combined one (which used to cause a `get_option` call
    // for an option that was never declared).
    let has_cxxstring_abi = package
        .platforms
        .iter()
        .any(|resolved| resolved.platform.triplet.cxxstring_abi.is_some());
    let has_libgfortran = package
        .platforms
        .iter()
        .any(|resolved| resolved.platform.triplet.libgfortran_version.is_some());

    let platforms = package
        .platforms
        .iter()
        .map(|resolved| PlatformSelector {
            identifier: resolved.platform.triplet.identifier(),
            condition: selector_condition(resolved),
        })
        .collect();

    let context = SelectorOverlayContext {
        name: &package.name,
        dependency_variable: dependency_variable_name.to_string(),
        needs_libc_probe,
        has_cxxstring_abi,
        has_libgfortran,
        platforms,
    };
    let rendered = render(&context, "selector_overlay.jinja")?;
    let path = subprojects_dir
        .join("packagefiles")
        .join(&package.name)
        .join("meson.build");
    write_generated_file(&path, &rendered, force)
}

/// Builds the full Meson boolean expression that matches `resolved`'s
/// triplet, so the template only ever has to interpolate one precomputed
/// string per platform.
fn selector_condition(resolved: &ResolvedPlatform) -> String {
    let triplet = &resolved.platform.triplet;
    let mut condition = format!(
        "host_machine.cpu_family() == '{}' and host_machine.system() == '{}'",
        triplet.arch.meson_cpu_family(),
        triplet.os.meson_system(),
    );
    if let Some(cpu) = triplet.arch.meson_cpu_disambiguator() {
        condition.push_str(&format!(" and host_machine.cpu() == '{cpu}'"));
    }
    if let Some(libc) = triplet.libc {
        condition.push_str(&format!(" and libc == '{}'", libc.identifier()));
    }
    if let Some(abi) = &triplet.cxxstring_abi {
        condition.push_str(&format!(" and cxxstring_abi == '{abi}'"));
    }
    if let Some(version) = &triplet.libgfortran_version {
        condition.push_str(&format!(" and libgfortran_version == '{version}'"));
    }
    condition
}

fn write_options(package: &JllPackage, subprojects_dir: &Path, force: bool) -> Result<()> {
    let cxxstring_values = distinct_sorted(
        package
            .platforms
            .iter()
            .filter_map(|resolved| resolved.platform.triplet.cxxstring_abi.clone()),
    );
    let libgfortran_values = distinct_sorted(
        package
            .platforms
            .iter()
            .filter_map(|resolved| resolved.platform.triplet.libgfortran_version.clone()),
    );

    // Most JLLs never split a platform by these ABI tags, so most packages
    // generate no `meson.options` at all.
    if cxxstring_values.is_empty() && libgfortran_values.is_empty() {
        return Ok(());
    }

    let cxxstring_default = cxxstring_values
        .iter()
        .find(|value| value.as_str() == "cxx11")
        .or_else(|| cxxstring_values.first())
        .cloned()
        .unwrap_or_default();
    let libgfortran_default = libgfortran_values
        .iter()
        .max_by_key(|value| value.parse::<u32>().unwrap_or(0))
        .cloned()
        .unwrap_or_default();

    let context = OptionsContext {
        has_cxxstring_abi: !cxxstring_values.is_empty(),
        cxxstring_choices: quoted_list(&cxxstring_values),
        cxxstring_default,
        has_libgfortran: !libgfortran_values.is_empty(),
        libgfortran_choices: quoted_list(&libgfortran_values),
        libgfortran_default,
    };
    let rendered = render(&context, "options.jinja")?;
    let path = subprojects_dir
        .join("packagefiles")
        .join(&package.name)
        .join("meson.options");
    write_generated_file(&path, &rendered, force)
}

fn distinct_sorted(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut values: Vec<String> = values.collect();
    values.sort();
    values.dedup();
    values
}

fn quoted_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render<T: Template>(context: &T, template_name: &'static str) -> Result<String> {
    context.render().map_err(|source| Error::Render {
        template: template_name,
        source,
    })
}

/// Writes `contents` to `path`, refusing to overwrite an existing file
/// unless `force` is set. Writes to a temporary file first and renames it
/// into place, so a failed write never leaves a half-written file behind.
fn write_generated_file(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(Error::AlreadyExists {
            path: path.to_path_buf(),
        });
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let temp_path = path.with_extension("tmp-meson-jll");
    fs::write(&temp_path, contents).map_err(|source| Error::WriteFile {
        path: temp_path.clone(),
        source,
    })?;
    fs::rename(&temp_path, path).map_err(|source| Error::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jll::artifacts::Platform;
    use crate::jll::triplet::{Arch, CallAbi, Libc, Os, Triplet};

    fn resolved(triplet: Triplet) -> ResolvedPlatform {
        ResolvedPlatform {
            platform: Platform {
                triplet,
                source_url: "https://example.invalid/archive.tar.gz".to_string(),
                source_hash: "0".repeat(64),
            },
            library_products: Vec::new(),
        }
    }

    #[test]
    fn armv6l_and_armv7l_conditions_are_distinguishable() {
        let armv6l = resolved(Triplet {
            arch: Arch::Armv6l,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: Some(CallAbi::HardFloat),
            cxxstring_abi: None,
            libgfortran_version: None,
        });
        let armv7l = resolved(Triplet {
            arch: Arch::Armv7l,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: Some(CallAbi::HardFloat),
            cxxstring_abi: None,
            libgfortran_version: None,
        });

        // Both report the same Meson cpu_family ("arm"), so without an
        // extra check on cpu() these two conditions would be identical and
        // the if-elif chain would always pick whichever came first.
        assert_ne!(selector_condition(&armv6l), selector_condition(&armv7l));
        assert!(selector_condition(&armv6l).contains("host_machine.cpu() == 'armv6l'"));
        assert!(selector_condition(&armv7l).contains("host_machine.cpu() == 'armv7l'"));
    }

    #[test]
    fn x86_64_condition_needs_no_disambiguator() {
        let platform = resolved(Triplet {
            arch: Arch::X86_64,
            os: Os::Linux,
            libc: Some(Libc::Glibc),
            call_abi: None,
            cxxstring_abi: None,
            libgfortran_version: None,
        });
        assert!(!selector_condition(&platform).contains("host_machine.cpu()"));
    }
}
