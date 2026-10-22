//! Runs the release wrap through a real Meson installation: package the
//! `meson-jll` binary the way a release does, generate the wrap archive and
//! `meson-jll.lock.wrap` from it, and check that `find_program('meson-jll')`
//! in a project without the tool on `PATH` extracts and runs that binary.
//!
//! The wrap points at GitHub release URLs that do not exist for this
//! build, so the test seeds Meson's package cache with the archives under
//! the names the wraps expect, and Meson never reaches the network. See
//! `scripts/release_wrap.py` for what is generated and why.
//!
//! Ignored by default because it needs Meson, Ninja, and Python on `PATH`.
//! Run explicitly with: `cargo test --test e2e_release_wrap -- --ignored`

use std::fs;
use std::path::Path;
use std::process::Command;

/// The release target the machine running this test is packaged as. The
/// binary under test is built for the host, so it is labeled with the
/// release target for the same operating system and architecture.
fn host_release_target() -> (&'static str, &'static str) {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("x86_64-unknown-linux-musl", "tar.gz"),
        ("linux", "aarch64") => ("aarch64-unknown-linux-musl", "tar.gz"),
        ("macos", "x86_64") => ("x86_64-apple-darwin", "tar.gz"),
        ("macos", "aarch64") => ("aarch64-apple-darwin", "tar.gz"),
        ("windows", "x86_64") => ("x86_64-pc-windows-msvc", "zip"),
        (os, arch) => panic!("There is no release target for {arch} on {os}."),
    }
}

/// Runs `scripts/release_wrap.py` with the given arguments and fails the
/// test with its output if it exits with an error.
fn run_release_script(arguments: &[&Path]) {
    let python = if cfg!(windows) { "python" } else { "python3" };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release_wrap.py");
    let mut command = Command::new(python);
    command.arg(script);
    for argument in arguments {
        command.arg(argument);
    }
    let output = command
        .output()
        .expect("Could not run python (is it on PATH?).");
    assert!(
        output.status.success(),
        "release_wrap.py failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "needs meson, ninja, and python on PATH"]
fn find_program_falls_back_to_the_release_binary() {
    let version = env!("CARGO_PKG_VERSION");
    let (triple, extension) = host_release_target();

    let workspace = tempfile::tempdir().expect("Could not create a workspace temp dir.");
    let archives_directory = workspace.path().join("archives");
    let output_directory = workspace.path().join("output");
    let cache_directory = workspace.path().join("cache");
    let consumer_directory = workspace.path().join("consumer");
    fs::create_dir_all(&cache_directory).unwrap();
    fs::create_dir_all(consumer_directory.join("subprojects")).unwrap();

    run_release_script(&[
        Path::new("package"),
        Path::new("--target"),
        Path::new(triple),
        Path::new("--binary"),
        Path::new(env!("CARGO_BIN_EXE_meson-jll")),
        Path::new("--output"),
        &archives_directory,
    ]);
    run_release_script(&[
        Path::new("wrap"),
        Path::new("--version"),
        Path::new(version),
        Path::new("--repository"),
        Path::new("nardi/meson-jll"),
        Path::new("--archives"),
        &archives_directory,
        Path::new("--output"),
        &output_directory,
        Path::new("--targets"),
        Path::new(triple),
    ]);

    // Seed the package cache under the file names the wraps ask for.
    fs::copy(
        archives_directory.join(format!("meson-jll-{triple}.{extension}")),
        cache_directory.join(format!("meson-jll-{version}-{triple}.{extension}")),
    )
    .unwrap();
    fs::copy(
        output_directory.join("meson-jll-wrap.tar.gz"),
        cache_directory.join(format!("meson-jll-wrap-{version}.tar.gz")),
    )
    .unwrap();

    // The lock sections that `meson-jll install` appends to the wrap must not
    // stop Meson from reading the wrap above them.
    let mut lock_wrap = fs::read_to_string(output_directory.join("meson-jll.lock.wrap")).unwrap();
    lock_wrap.push_str(
        "\n[meson-jll]\nformat = 1\n\n[meson-jll.roots]\nExampleThing = *\n\n\
         [meson-jll.package.ExampleThing]\nversion = 1.0.0+0\n",
    );
    fs::write(
        consumer_directory.join("subprojects/meson-jll.lock.wrap"),
        lock_wrap,
    )
    .unwrap();

    fs::write(
        consumer_directory.join("meson.build"),
        format!(
            "project('e2e-release-wrap')\n\
             meson_jll = find_program('meson-jll')\n\
             assert(meson_jll.full_path().contains('subprojects'))\n\
             version = run_command(meson_jll, '--version', check: true)\n\
             assert(version.stdout().contains('{version}'))\n"
        ),
    )
    .unwrap();

    let setup = Command::new("meson")
        .arg("setup")
        .arg("build")
        .current_dir(&consumer_directory)
        .env("MESON_PACKAGE_CACHE_DIR", &cache_directory)
        .output()
        .expect("Could not run meson setup (is meson on PATH?).");
    assert!(
        setup.status.success(),
        "meson setup failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&setup.stdout),
        String::from_utf8_lossy(&setup.stderr)
    );
}
