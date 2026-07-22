//! Runs the full pipeline against real, published JLL tarballs, unlike
//! `e2e_meson.rs`, which builds and tars up a synthetic library instead.
//!
//! A bug can hide in what a real tarball actually contains (the fix in
//! `4be3533`, macOS `libgcc_s` with no dev symlink, was invisible to every
//! other test since none of them ever downloaded a real
//! CompilerSupportLibraries tarball). Both fixtures here pull it in
//! transitively, so that fallback path is exercised for free.
//!
//! The two fixture projects live in `tests/projects/`, each pinned by its
//! own `meson-jll.lock`. `sync` regenerates wraps straight from that
//! lockfile without resolving anything, which also keeps it off
//! `api.github.com`'s rate limit (only `git ls-remote` and an archive
//! download are used).
//!
//! Ignored by default, needing `meson`, `ninja`, a C compiler, and (for the
//! wheel test) `pip` and `meson-python` on `PATH`, plus network access. Run
//! explicitly with: `cargo test --test e2e_real_jll -- --ignored`

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::{assert_success, copy_dir_recursive};

/// The repo-relative directory holding both vendored fixture projects.
fn projects_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/projects"))
}

/// Copies `project_name` (a subdirectory of `tests/projects/`) into a fresh
/// temp directory, so a test run never writes into the checked-in fixture
/// and always starts from the same clean state.
fn stage_project(project_name: &str) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().expect("could not create a workspace temp dir");
    copy_dir_recursive(&projects_dir().join(project_name), workspace.path());
    workspace
}

/// Runs `meson-jll sync` against `project_dir`'s committed lockfile,
/// writing wraps into `project_dir/subprojects`.
fn sync(project_dir: &Path) {
    let sync = Command::new(env!("CARGO_BIN_EXE_meson-jll"))
        .arg("sync")
        .arg("--subprojects-dir")
        .arg(project_dir.join("subprojects"))
        .output()
        .expect("could not run the meson-jll binary");
    assert_success(&sync, "meson-jll sync");
}

/// Where `meson install` actually put every installed file, straight from
/// Meson's own introspection rather than a guessed `<prefix>/<libdir>` join
/// (Debian's multiarch `lib/x86_64-linux-gnu` default, on `ubuntu-latest`
/// CI runners, would break a hardcoded `lib` guess).
fn installed_paths(build_dir: &Path) -> Vec<PathBuf> {
    let introspect = Command::new("meson")
        .args(["introspect", "--installed"])
        .arg(build_dir)
        .output()
        .expect("could not run meson introspect");
    assert_success(&introspect, "meson introspect --installed");

    let map: serde_json::Value =
        serde_json::from_slice(&introspect.stdout).expect("meson introspect did not print JSON");
    map.as_object()
        .expect("meson introspect --installed should be a JSON object")
        .values()
        .map(|install_path| {
            PathBuf::from(
                install_path
                    .as_str()
                    .expect("each installed value should be a path string"),
            )
        })
        .collect()
}

/// The platform's shared library search path variable: `PATH` on Windows
/// (no other mechanism exists), `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH`
/// elsewhere (an installed binary's rpath is not guaranteed to reach here,
/// unlike the build-tree rpath Meson sets up automatically).
fn library_search_path_env_var() -> &'static str {
    if cfg!(windows) {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

/// Prepends `dirs` onto whatever this process already has set for `var`.
/// Must be an addition, not a replacement, on Windows: `PATH` is also
/// where the MSVC runtime DLLs live, so replacing it outright would leave
/// the demo executable unable to start at all.
fn prepend_to_env_path(var: &str, dirs: Vec<PathBuf>) -> OsString {
    let mut paths = dirs;
    if let Some(existing) = env::var_os(var) {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths).expect("could not build a library search path value")
}

#[test]
#[ignore = "needs meson, ninja, a C compiler, and network access"]
fn suitesparse_links_and_runs_against_real_tarballs() {
    let workspace = stage_project("suitesparse");
    let project_dir = workspace.path();
    sync(project_dir);

    let build_dir = project_dir.join("build");
    let install_dir = project_dir.join("install");
    let setup = Command::new("meson")
        .arg("setup")
        .arg(&build_dir)
        .arg("--prefix")
        .arg(&install_dir)
        .current_dir(project_dir)
        .output()
        .expect("could not run meson setup (is meson on PATH?)");
    assert_success(&setup, "meson setup");

    let compile = Command::new("meson")
        .args(["compile", "-C"])
        .arg(&build_dir)
        .output()
        .expect("could not run meson compile");
    assert_success(&compile, "meson compile");

    // Installed, not run straight from the build directory: a plain C
    // executable has no way of its own to locate a shared library at
    // runtime the way highs_wheel's Python package can (its __init__.py
    // adds a DLL directory). This is what a real consumer would do too.
    let install = Command::new("meson")
        .args(["install", "-C"])
        .arg(&build_dir)
        .output()
        .expect("could not run meson install");
    assert_success(&install, "meson install");

    let demo_name = if cfg!(windows) { "demo.exe" } else { "demo" };
    let installed = installed_paths(&build_dir);
    let demo_path = installed
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name == demo_name))
        .unwrap_or_else(|| panic!("{demo_name} was not among the installed files: {installed:?}"));
    // `--installed`'s value is the directory itself for a whole-directory
    // install (install_subdir), but the file's own path for a single-file
    // install (the demo executable). Only the latter needs `.parent()`.
    let library_dirs: Vec<PathBuf> = installed
        .iter()
        .filter(|path| *path != demo_path)
        .map(|path| {
            if path.is_dir() {
                path.clone()
            } else {
                path.parent().unwrap_or(path).to_path_buf()
            }
        })
        .collect();
    let library_search_path_var = library_search_path_env_var();
    let library_search_path = prepend_to_env_path(library_search_path_var, library_dirs);

    let run = Command::new(demo_path)
        .env(library_search_path_var, &library_search_path)
        .output()
        .expect("could not run the installed demo executable");
    assert_success(&run, "the installed demo executable");

    // Proves the executable actually linked against, and can call into,
    // SuiteSparse_config at runtime, not just that the link step succeeded.
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.starts_with("SuiteSparse "),
        "unexpected demo output: {stdout}"
    );
}

/// The venv's `python`, and its own script directory (needed on `PATH` so
/// the `meson` and `ninja` installed into this same venv are found by the
/// `--no-build-isolation` build).
struct Venv {
    python: PathBuf,
    scripts_dir: PathBuf,
}

fn create_venv(venv_dir: &Path) -> Venv {
    // Whatever `python` resolves to on PATH creates the venv, then
    // everything else the build needs is installed into it explicitly.
    let create = Command::new("python")
        .args(["-m", "venv"])
        .arg(venv_dir)
        .output()
        .expect("could not run python (is it on PATH?)");
    assert_success(&create, "python -m venv");

    let (python, scripts_dir) = if cfg!(windows) {
        (
            venv_dir.join("Scripts/python.exe"),
            venv_dir.join("Scripts"),
        )
    } else {
        (venv_dir.join("bin/python"), venv_dir.join("bin"))
    };
    Venv {
        python,
        scripts_dir,
    }
}

/// Prepends the venv's script directory to `PATH`, so its `meson` and
/// `ninja` entry points are found ahead of any other copy on the system.
fn path_with_venv_scripts(venv: &Venv) -> OsString {
    prepend_to_env_path("PATH", vec![venv.scripts_dir.clone()])
}

#[test]
#[ignore = "needs meson, ninja, a C compiler, pip, and network access"]
fn highs_wheel_builds_and_imports_against_real_tarballs() {
    let workspace = stage_project("highs_wheel");
    let project_dir = workspace.path();
    sync(project_dir);

    let venv = create_venv(&project_dir.join(".venv"));
    let path = path_with_venv_scripts(&venv);

    let install_build_deps = Command::new(&venv.python)
        .args(["-m", "pip", "install", "meson", "ninja", "meson-python"])
        .env("PATH", &path)
        .output()
        .expect("could not run pip install");
    assert_success(&install_build_deps, "pip install meson ninja meson-python");

    // --no-build-isolation: build against the JLL subprojects already
    // synced here, not a second isolated copy pip would otherwise fetch.
    let install = Command::new(&venv.python)
        .args(["-m", "pip", "install", ".", "--no-build-isolation"])
        .current_dir(project_dir)
        .env("PATH", &path)
        .output()
        .expect("could not run pip install .");
    assert_success(&install, "pip install . --no-build-isolation");

    // The real proof: import the built extension and call into HiGHS,
    // which only works if the bundled shared libraries were found.
    let run = Command::new(&venv.python)
        .args([
            "-c",
            "import demo_ext; demo_ext.create_and_destroy(); print(demo_ext.version())",
        ])
        .env("PATH", &path)
        .output()
        .expect("could not run python -c");
    assert_success(&run, "python -c 'import demo_ext; ...'");

    // Also build a wheel and check its listing for the bundled libraries,
    // so a regression that stops bundling them fails loudly.
    let dist_dir = project_dir.join("dist");
    let wheel = Command::new(&venv.python)
        .args(["-m", "pip", "wheel", ".", "--no-build-isolation", "-w"])
        .arg(&dist_dir)
        .current_dir(project_dir)
        .env("PATH", &path)
        .output()
        .expect("could not run pip wheel");
    assert_success(&wheel, "pip wheel . --no-build-isolation");

    let wheel_path = fs::read_dir(&dist_dir)
        .expect("could not read the dist directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .expect("pip wheel did not produce a .whl file");

    // Inspected via Python's own zipfile module rather than adding a Rust
    // zip crate this codebase has no other use for: a wheel is just a zip.
    let check_script = format!(
        "import zipfile\n\
         names = zipfile.ZipFile(r'{wheel}').namelist()\n\
         bundled = [n for n in names if '.mesonpy.libs' in n]\n\
         assert bundled, f'no bundled shared libraries found in the wheel: {{names}}'\n\
         assert any('libhighs' in n.lower() for n in bundled), \\\n\
             f'libhighs missing from the bundled libraries: {{bundled}}'\n",
        wheel = wheel_path.display(),
    );
    let check = Command::new(&venv.python)
        .args(["-c", &check_script])
        .env("PATH", &path)
        .output()
        .expect("could not run the wheel-contents check script");
    assert_success(&check, "wheel-contents check");
}
