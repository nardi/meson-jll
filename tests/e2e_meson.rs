//! Runs the full pipeline against a real Meson installation: install a wrap
//! set generated from a small, genuinely compiled library, then `meson
//! setup`, `meson compile`, and run the result.
//!
//! Every other test in this crate proves the generator produces the text
//! we intend. This one proves that text is actually valid input to Meson,
//! which caught bugs (a source-less `[wrap-file]` and a missing
//! `patch_directory`) that no amount of string-matching against the
//! generated output would have. See the crate's CI configuration for how
//! this is run, since it needs `meson`, `ninja`, and a C compiler on PATH,
//! none of which a plain `cargo test` run can assume.
//!
//! Ignored by default. Run explicitly with:
//! `cargo test --test e2e_meson -- --ignored`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use meson_jll::jll::triplet::{Arch, Libc, Os, Triplet};
use sha2::{Digest, Sha256};

mod common;
use common::assert_success;

/// The Julia-side platform selectors for the machine running this test, in
/// the vocabulary `Artifacts.toml` itself uses (`x86_64`/`aarch64`,
/// `linux`/`macos`/`windows`, `glibc`). Kept as plain strings independent
/// of the crate's own `Os`/`Arch` parsing, so this test exercises that
/// parsing rather than assuming it.
struct HostPlatform {
    julia_arch: &'static str,
    julia_os: &'static str,
    julia_libc: Option<&'static str>,
}

fn host_platform() -> HostPlatform {
    let julia_arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => panic!("e2e_meson does not know the Julia arch string for {other}"),
    };
    let julia_os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => panic!("e2e_meson does not know the Julia os string for {other}"),
    };
    let julia_libc = (julia_os == "linux").then_some("glibc");
    HostPlatform {
        julia_arch,
        julia_os,
        julia_libc,
    }
}

/// Compiles `example.c` into a real, minimal static library for the host,
/// using whatever compiler is already on PATH (`cl` on Windows, `cc`
/// elsewhere). Returns its path and file name.
///
/// A real archive is used, not a placeholder, because the point of this
/// test is to prove Meson's `cc.find_library()` and the final link step
/// actually succeed against what the generator produces.
fn compile_static_library(build_dir: &Path) -> (PathBuf, &'static str) {
    let source_path = build_dir.join("example.c");
    fs::write(
        &source_path,
        "int example_unused_symbol(void) { return 0; }\n",
    )
    .expect("could not write example.c");

    if cfg!(windows) {
        let status = Command::new("cl")
            .args(["/nologo", "/c"])
            .arg(&source_path)
            .current_dir(build_dir)
            .status()
            .expect("could not run cl.exe (is a Visual Studio dev environment active?)");
        assert!(status.success(), "cl.exe failed to compile example.c");

        let status = Command::new("lib")
            .args(["/nologo", "/out:example.lib", "example.obj"])
            .current_dir(build_dir)
            .status()
            .expect("could not run lib.exe");
        assert!(status.success(), "lib.exe failed to archive example.obj");

        (build_dir.join("example.lib"), "example.lib")
    } else {
        let object_path = build_dir.join("example.o");
        let status = Command::new("cc")
            .arg("-c")
            .arg(&source_path)
            .arg("-o")
            .arg(&object_path)
            .status()
            .expect("could not run cc");
        assert!(status.success(), "cc failed to compile example.c");

        let status = Command::new("ar")
            .args(["rcs", "libexample.a"])
            .arg(&object_path)
            .current_dir(build_dir)
            .status()
            .expect("could not run ar");
        assert!(status.success(), "ar failed to archive example.o");

        (build_dir.join("libexample.a"), "libexample.a")
    }
}

/// Packages `library_path` into a flat `.tar.gz`, the same layout JLL
/// releases use, and returns the archive's path and sha256.
fn package_tarball(
    workspace: &Path,
    tarball_name: &str,
    library_path: &Path,
    library_file_name: &str,
) -> (PathBuf, String) {
    let tarball_path = workspace.join(tarball_name);
    {
        let file = fs::File::create(&tarball_path).expect("could not create tarball");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_path_with_name(library_path, format!("lib/{library_file_name}"))
            .expect("could not append the compiled library to the tarball");
        builder
            .into_inner()
            .expect("could not finish the tar stream")
            .finish()
            .expect("could not finish the gzip stream");
    }
    let tarball_bytes = fs::read(&tarball_path).expect("could not read the tarball back");
    let source_hash = format!("{:x}", Sha256::digest(&tarball_bytes));
    (tarball_path, source_hash)
}

#[test]
#[ignore = "needs meson, ninja, and a C compiler on PATH"]
fn installs_and_builds_against_real_meson() {
    let platform = host_platform();
    let triplet = Triplet {
        arch: Arch::parse(platform.julia_arch).expect("host arch should parse"),
        os: Os::parse(platform.julia_os).expect("host os should parse"),
        libc: platform.julia_libc.and_then(Libc::parse),
        call_abi: None,
        cxxstring_abi: None,
        libgfortran_version: None,
    };
    let identifier = triplet.identifier();

    let workspace = tempfile::tempdir().expect("could not create a workspace temp dir");
    let fixture_dir = workspace.path().join("fixture");
    let consumer_dir = workspace.path().join("consumer");
    fs::create_dir_all(fixture_dir.join("src/wrappers")).unwrap();
    fs::create_dir_all(consumer_dir.join("subprojects/packagecache")).unwrap();

    let (library_path, library_file_name) = compile_static_library(workspace.path());
    let tarball_name = format!("ExampleThing.v1.0.0.{identifier}.tar.gz");
    let (tarball_path, source_hash) = package_tarball(
        workspace.path(),
        &tarball_name,
        &library_path,
        library_file_name,
    );

    fs::write(
        fixture_dir.join("Project.toml"),
        r#"name = "ExampleThing_jll"
uuid = "00000000-0000-0000-0000-000000000000"
version = "1.0.0+0"

[deps]
JLLWrappers = "692b3bcd-3c85-4b1f-b108-f13ce0eb3210"
"#,
    )
    .unwrap();

    let libc_line = platform
        .julia_libc
        .map(|libc| format!("libc = \"{libc}\"\n"))
        .unwrap_or_default();
    fs::write(
        fixture_dir.join("Artifacts.toml"),
        format!(
            r#"[[ExampleThing]]
arch = "{arch}"
os = "{os}"
{libc_line}
    [[ExampleThing.download]]
    url = "https://example.invalid/{tarball_name}"
    sha256 = "{source_hash}"
"#,
            arch = platform.julia_arch,
            os = platform.julia_os,
        ),
    )
    .unwrap();

    fs::write(
        fixture_dir.join(format!("src/wrappers/{identifier}.jl")),
        format!(
            r#"using JLLWrappers

export libexample

JLLWrappers.@declare_library_product(libexample, "libexample.soname")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/{library_file_name}",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
"#
        ),
    )
    .unwrap();

    // Seed the package cache so Meson never has to reach the (fake) URL.
    fs::copy(
        &tarball_path,
        consumer_dir
            .join("subprojects/packagecache")
            .join(&tarball_name),
    )
    .unwrap();

    let install = Command::new(env!("CARGO_BIN_EXE_meson-jll"))
        .args(["install", "ExampleThing", "--url"])
        .arg(&fixture_dir)
        .arg("--subprojects-dir")
        .arg(consumer_dir.join("subprojects"))
        .output()
        .expect("could not run the meson-jll binary");
    assert_success(&install, "meson-jll install");

    fs::write(
        consumer_dir.join("meson.build"),
        "project('e2e-demo', 'c')\n\
         example = dependency('ExampleThing_jll')\n\
         executable('demo', 'demo.c', dependencies: example)\n",
    )
    .unwrap();
    fs::write(
        consumer_dir.join("demo.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();

    let build_dir = consumer_dir.join("build");
    let setup = Command::new("meson")
        .arg("setup")
        .arg(&build_dir)
        .current_dir(&consumer_dir)
        .output()
        .expect("could not run meson setup (is meson on PATH?)");
    assert_success(&setup, "meson setup");

    let compile = Command::new("meson")
        .args(["compile", "-C"])
        .arg(&build_dir)
        .output()
        .expect("could not run meson compile");
    assert_success(&compile, "meson compile");

    let demo_name = if cfg!(windows) { "demo.exe" } else { "demo" };
    let run_status = Command::new(build_dir.join(demo_name))
        .status()
        .expect("could not run the built demo executable");
    assert!(
        run_status.success(),
        "the built demo executable exited with a failure"
    );
}
