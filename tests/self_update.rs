//! Runs `meson-jll self-update` against a wrap file read from disk.
//!
//! The command reads the wrap of a release from GitHub by default. These
//! tests point `MESON_JLL_WRAP` at a local file instead, so they run offline
//! and can use any version.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use meson_jll::install::lock_path_for;
use meson_jll::lock::{LockFile, LockedPackage};

fn wrap_of(version: &str) -> String {
    format!(
        "[wrap-file]\ndirectory = meson-jll-wrap-{version}\n\n[provide]\nprogram_names = meson-jll\n"
    )
}

fn self_update(project: &Path, wrap: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_meson-jll"))
        .arg("self-update")
        .args(arguments)
        .arg("--subprojects-dir")
        .arg(project.join("subprojects"))
        .env("MESON_JLL_WRAP", wrap)
        .output()
        .expect("Could not run the meson-jll binary.")
}

fn standard_output(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn moves_a_project_to_another_release_and_keeps_its_lock() {
    let project = tempfile::tempdir().unwrap();
    let path = lock_path_for(&project.path().join("subprojects"));
    let lock = LockFile {
        roots: [("ExampleThing".to_string(), "*".to_string())].into(),
        packages: vec![LockedPackage {
            name: "ExampleThing".to_string(),
            version: "1.2.3+0".to_string(),
            dependencies: vec![],
        }],
    };
    lock.write_starting_with(&path, Some(&wrap_of("0.1.0")))
        .unwrap();
    let new_wrap = project.path().join("new.wrap");
    fs::write(&new_wrap, wrap_of("0.2.0")).unwrap();

    let output = self_update(project.path(), &new_wrap, &[]);
    assert!(output.status.success(), "{output:?}");
    let text = standard_output(&output);
    assert!(text.contains("0.1.0") && text.contains("0.2.0"), "{text}");
    let file = fs::read_to_string(&path).unwrap();
    assert!(file.starts_with(&wrap_of("0.2.0")));
    assert_eq!(LockFile::read(&path).unwrap().packages, lock.packages);

    let output = self_update(project.path(), &new_wrap, &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(standard_output(&output).contains("already pinned"));
}

#[test]
fn pins_a_release_in_a_project_without_a_wrap_file() {
    let project = tempfile::tempdir().unwrap();
    let new_wrap = project.path().join("new.wrap");
    fs::write(&new_wrap, wrap_of("0.2.0")).unwrap();

    let output = self_update(project.path(), &new_wrap, &["0.2.0"]);

    assert!(output.status.success(), "{output:?}");
    assert!(standard_output(&output).contains("Pinned"));
    let path = lock_path_for(&project.path().join("subprojects"));
    assert_eq!(fs::read_to_string(path).unwrap(), wrap_of("0.2.0"));
}

#[test]
fn refuses_a_file_that_is_not_the_wrap_of_a_release() {
    let project = tempfile::tempdir().unwrap();
    let not_a_wrap = project.path().join("not.wrap");
    fs::write(&not_a_wrap, "[wrap-file]\ndirectory = something-else\n").unwrap();

    let output = self_update(project.path(), &not_a_wrap, &[]);

    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("is not a wrap that provides meson-jll"),
        "{message}"
    );
    assert!(!lock_path_for(&project.path().join("subprojects")).exists());
}
