//! Small helpers shared by the end-to-end test files (`e2e_meson.rs` and
//! `e2e_real_jll.rs`), both of which drive the built `meson-jll` binary and
//! real `meson`/`pip` invocations rather than calling this crate's own
//! functions directly.
//!
//! Each integration test file is its own separate binary, and `mod
//! common;` compiles this module fresh into every one of them, so a
//! helper only one of the two files calls looks unused from the other
//! binary's point of view. `#![allow(dead_code)]` covers that, rather than
//! `#[allow]` on each individual function as the set of callers shifts.
#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Output;

/// Asserts `output`'s process exited successfully, printing both stdout and
/// stderr on failure. Every external command in an e2e test (`meson-jll`,
/// `meson`, `pip`, the built demo executable) is checked this way, since a
/// bare `status.success()` assertion leaves a CI failure with no output to
/// diagnose from.
pub fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Recursively copies `src` into `dst`, creating `dst` if needed. Used to
/// stage a committed fixture project into a fresh temp directory before
/// running `meson-jll` and `meson` against it, so a test run never dirties
/// the checked-in fixture and always starts from the same clean state.
pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("could not create destination directory");
    for entry in fs::read_dir(src).expect("could not read source directory") {
        let entry = entry.expect("could not read directory entry");
        let file_type = entry.file_type().expect("could not read file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            fs::copy(entry.path(), &dst_path).expect("could not copy file");
        }
    }
}
