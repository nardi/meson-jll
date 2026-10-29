//! Checks which shared libraries an installed JLL ends up with, using a
//! synthetic JLL built from real shared libraries and a real `meson
//! install`.
//!
//! A wheel is whatever `meson install` puts in `libdir`, so the install
//! rules decide whether the wheel imports on a machine without the build
//! tree. The fixture has the shapes a real JLL tarball has that broke
//! earlier versions of those rules:
//!
//! - a library with a versioned soname that Julia's wrapper names through an
//!   unversioned development symlink (`libexample.so` for `libexample.so.3`),
//! - a library the first one needs that no wrapper declares (`libhidden`),
//! - a declared library below a subdirectory of `lib/`, as METIS has,
//! - a library nothing declares or needs (`libunused`), standing in for the
//!   sanitizer runtimes in CompilerSupportLibraries.
//!
//! The installed `libdir` must hold exactly the three libraries the loader
//! asks for, under the names it asks for them, and on Linux the installed
//! demo must run from there alone. The demo must also run from the build
//! tree without any linker flag in the consumer, although the first library
//! needs the nested one from a directory its own run path does not cover.
//!
//! Ignored by default because it needs Meson, Ninja, and a C compiler on
//! `PATH`. Run explicitly with:
//! `cargo test --test e2e_runtime_libraries -- --ignored`
#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use meson_jll::jll::triplet::{Arch, Libc, Os, Triplet};
use sha2::{Digest, Sha256};

mod common;
use common::{assert_success, host_platform};

/// The file name of a shared library on the host, for example
/// `libexample.so.3` on Linux and `libexample.3.dylib` on macOS.
fn shared_library_name(base: &str, version: Option<&str>) -> String {
    match (cfg!(target_os = "macos"), version) {
        (true, Some(version)) => format!("lib{base}.{version}.dylib"),
        (true, None) => format!("lib{base}.dylib"),
        (false, Some(version)) => format!("lib{base}.so.{version}"),
        (false, None) => format!("lib{base}.so"),
    }
}

/// Compiles `source` into a shared library in `directory` whose soname (or
/// install name on macOS) is its own file name, and returns that file name.
/// `link_against` names libraries to link by their unversioned development
/// name, each with the directory that holds it.
fn compile_shared_library(
    directory: &Path,
    base: &str,
    version: Option<&str>,
    source: &str,
    link_against: &[(&Path, &str)],
) -> String {
    let file_name = shared_library_name(base, version);
    let source_path = directory.join(format!("{base}.c"));
    fs::write(&source_path, source).expect("could not write the library source");

    let mut command = Command::new("cc");
    if cfg!(target_os = "macos") {
        command.args([
            "-dynamiclib",
            "-install_name",
            &format!("@rpath/{file_name}"),
            "-Wl,-rpath,@loader_path",
        ]);
    } else {
        command.args([
            "-shared",
            "-fPIC",
            &format!("-Wl,-soname,{file_name}"),
            "-Wl,-rpath,$ORIGIN",
        ]);
    }
    command.arg(&source_path).arg("-o").arg(&file_name);
    for (library_directory, library) in link_against {
        command.arg(format!("-L{}", library_directory.display()));
        command.arg(format!("-l{library}"));
    }
    let output = command
        .current_dir(directory)
        .output()
        .expect("could not run cc");
    assert_success(&output, &format!("cc for {file_name}"));
    file_name
}

/// Appends a symbolic link entry to a tarball.
fn append_symlink(builder: &mut tar::Builder<impl std::io::Write>, path: &str, target: &str) {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    builder
        .append_link(&mut header, path, target)
        .expect("could not append a symlink to the tarball");
}

#[test]
#[ignore = "needs meson, ninja, and a C compiler on PATH"]
fn installs_the_libraries_the_loader_asks_for() {
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
    // Julia names a wrapper script by its own convention, which differs from
    // the one of the generated files on macOS.
    let wrapper_identifier = triplet.julia_wrapper_identifier();

    let workspace = tempfile::tempdir().expect("could not create a workspace temp dir");
    let libraries_dir = workspace.path().join("libraries");
    let nested_dir = workspace.path().join("nested");
    let fixture_dir = workspace.path().join("fixture");
    let consumer_dir = workspace.path().join("consumer");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::create_dir_all(&nested_dir).unwrap();
    fs::create_dir_all(fixture_dir.join("src/wrappers")).unwrap();
    fs::create_dir_all(consumer_dir.join("subprojects/packagecache")).unwrap();

    // The hidden library has to exist as a development name before the
    // library that needs it can link against it.
    let hidden = compile_shared_library(
        &libraries_dir,
        "hidden",
        Some("1"),
        "int hidden_value(void) { return 7; }\n",
        &[],
    );
    let hidden_development = shared_library_name("hidden", None);
    symlink(&hidden, libraries_dir.join(&hidden_development)).unwrap();

    let nested = compile_shared_library(
        &nested_dir,
        "nested",
        None,
        "int nested_value(void) { return 3; }\n",
        &[],
    );

    // The example library needs the nested one, which sits in a directory
    // its own run path does not cover, as a library of another JLL would.
    let example = compile_shared_library(
        &libraries_dir,
        "example",
        Some("3"),
        "int hidden_value(void);\nint nested_value(void);\n\
         int example_value(void) { return hidden_value() + nested_value(); }\n",
        &[(&libraries_dir, "hidden"), (&nested_dir, "nested")],
    );
    let example_development = shared_library_name("example", None);
    symlink(&example, libraries_dir.join(&example_development)).unwrap();

    let unused = compile_shared_library(
        &libraries_dir,
        "unused",
        None,
        "int unused_value(void) { return 0; }\n",
        &[],
    );

    let tarball_name = format!("ExampleThing.v1.0.0.{identifier}.tar.gz");
    let tarball_path = workspace.path().join(&tarball_name);
    {
        let file = fs::File::create(&tarball_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_path_with_name(libraries_dir.join(&example), format!("lib/{example}"))
            .unwrap();
        append_symlink(
            &mut builder,
            &format!("lib/{example_development}"),
            &example,
        );
        builder
            .append_path_with_name(libraries_dir.join(&hidden), format!("lib/{hidden}"))
            .unwrap();
        append_symlink(&mut builder, &format!("lib/{hidden_development}"), &hidden);
        builder
            .append_path_with_name(libraries_dir.join(&unused), format!("lib/{unused}"))
            .unwrap();
        builder
            .append_path_with_name(
                nested_dir.join(&nested),
                format!("lib/nested/variant/lib/{nested}"),
            )
            .unwrap();
        builder
            .into_inner()
            .unwrap()
            .finish()
            .expect("could not finish the tarball");
    }
    let source_hash = format!("{:x}", Sha256::digest(fs::read(&tarball_path).unwrap()));
    fs::copy(
        &tarball_path,
        consumer_dir
            .join("subprojects/packagecache")
            .join(&tarball_name),
    )
    .unwrap();

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
        fixture_dir.join(format!("src/wrappers/{wrapper_identifier}.jl")),
        format!(
            r#"using JLLWrappers

export libexample, libnested

JLLWrappers.@declare_library_product(libexample, "{example}")
JLLWrappers.@declare_library_product(libnested, "{nested}")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/{example_development}",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
    JLLWrappers.@init_library_product(
        libnested,
        "lib/nested/variant/lib/{nested}",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
"#
        ),
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
        "project('e2e-runtime-libraries', 'c')\n\
         example = dependency('ExampleThing_jll')\n\
         executable('demo', 'demo.c', dependencies: example, install: true)\n",
    )
    .unwrap();
    fs::write(
        consumer_dir.join("demo.c"),
        "int example_value(void);\n\
         int main(void) { return example_value() == 10 ? 0 : 1; }\n",
    )
    .unwrap();

    let build_dir = consumer_dir.join("build");
    let destination = workspace.path().join("destination");
    let setup = Command::new("meson")
        .args(["setup", "-Dprefix=/usr", "-Dlibdir=lib"])
        .arg(&build_dir)
        .current_dir(&consumer_dir)
        .output()
        .expect("could not run meson setup (is meson on PATH?)");
    assert_success(&setup, "meson setup");

    // On Linux the demo also has to run straight from the build tree, with
    // no linker flag of its own. It calls only the example library, which
    // needs one from a directory its run path does not cover, so the loader
    // can only find that one if the demo records it as needed itself.
    if cfg!(target_os = "linux") {
        let compile = Command::new("meson")
            .args(["compile", "-C"])
            .arg(&build_dir)
            .output()
            .expect("could not run meson compile");
        assert_success(&compile, "meson compile");
        let run = Command::new(build_dir.join("demo"))
            .env_remove("LD_LIBRARY_PATH")
            .output()
            .expect("could not run the demo from the build tree");
        assert_success(&run, "the demo run from the build tree");
    }

    let install = Command::new("meson")
        .args(["install", "-C"])
        .arg(&build_dir)
        .arg("--destdir")
        .arg(&destination)
        .output()
        .expect("could not run meson install");
    assert_success(&install, "meson install");

    // Exactly these three, flat in libdir and under the names the loader
    // records: no development symlinks, no copy of the unused library.
    let installed_dir: PathBuf = destination.join("usr/lib");
    let installed: BTreeSet<String> = fs::read_dir(&installed_dir)
        .expect("could not read the installed libdir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let expected: BTreeSet<String> = [example.clone(), hidden.clone(), nested.clone()]
        .into_iter()
        .collect();
    assert_eq!(
        installed, expected,
        "unexpected libraries in the installed libdir (the unused one was {unused})"
    );

    // Only Linux is run from the installed tree: the demo's own run path
    // does not survive `meson install`, so it finds the libraries through
    // `LD_LIBRARY_PATH` alone, which is the loader asking for them by name.
    if cfg!(target_os = "linux") {
        let run = Command::new(destination.join("usr/bin/demo"))
            .env("LD_LIBRARY_PATH", &installed_dir)
            .output()
            .expect("could not run the installed demo");
        assert_success(&run, "the installed demo");
    }
}
