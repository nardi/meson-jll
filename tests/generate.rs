//! End-to-end test of the generator, from a vendored fixture JLL all the
//! way to the generated files on disk. This never touches the network:
//! `tests/fixtures/example_jll` is a small hand-written stand-in for a real
//! JLL repository, read through the same `Source` a `--url` local path
//! would use.

use std::fs;

use meson_jll::generate;
use meson_jll::jll;
use meson_jll::source::CustomSource;
use meson_jll::Error;

fn fixture_source() -> CustomSource {
    let fixture_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/example_jll");
    CustomSource::parse(fixture_dir, "unused")
}

#[test]
fn generates_a_full_wrap_set_from_a_fixture_jll() {
    let source = fixture_source();
    let package = jll::load(&source).expect("fixture metadata should parse");

    assert_eq!(package.name, "ExampleThing");
    assert_eq!(package.version, "1.2.3+0");
    assert_eq!(package.platforms.len(), 2);
    assert!(package.dependencies.is_empty());

    let output_dir = tempfile::tempdir().expect("could not create a temp dir");
    generate::write_wrap_set(&package, output_dir.path(), false)
        .expect("generation should succeed");

    let selector_wrap = fs::read_to_string(output_dir.path().join("ExampleThing.wrap")).unwrap();
    assert!(selector_wrap.contains("# meson-jll: name=ExampleThing version=1.2.3+0"));
    assert!(selector_wrap.contains("directory = ExampleThing"));
    assert!(selector_wrap.contains("dependency_names = ExampleThing_jll"));
    // Meson does not discover packagefiles/<name>/ on its own: without this,
    // the overlay is silently never applied.
    assert!(selector_wrap.contains("patch_directory = ExampleThing"));
    // A source-less [wrap-file] is rejected by Meson outright: it always
    // requires a source_filename, even for an overlay-only wrap.
    assert!(selector_wrap.contains(&format!(
        "source_filename = {}",
        generate::EMPTY_TAR_FILENAME
    )));
    assert!(output_dir
        .path()
        .join("packagefiles")
        .join(generate::EMPTY_TAR_FILENAME)
        .exists());

    let selector_overlay = fs::read_to_string(
        output_dir
            .path()
            .join("packagefiles/ExampleThing/meson.build"),
    )
    .unwrap();
    assert!(selector_overlay.contains(
        "host_machine.cpu_family() == 'x86_64' and host_machine.system() == 'linux' and libc == 'gnu'"
    ));
    assert!(selector_overlay
        .contains("host_machine.cpu_family() == 'aarch64' and host_machine.system() == 'darwin'"));
    assert!(selector_overlay.contains("subproject('ExampleThing-' + triplet)"));
    assert!(selector_overlay
        .contains("meson.override_dependency('ExampleThing_jll', examplething_dep)"));

    let linux_wrap =
        fs::read_to_string(output_dir.path().join("ExampleThing-x86_64-linux-gnu.wrap")).unwrap();
    assert!(linux_wrap.contains(
        "source_url = https://example.invalid/ExampleThing.v1.2.3.x86_64-linux-gnu.tar.gz"
    ));
    assert!(linux_wrap.contains(
        "source_hash = 1111111111111111111111111111111111111111111111111111111111111111"
    ));
    assert!(linux_wrap.contains("patch_directory = ExampleThing-x86_64-linux-gnu"));

    let linux_overlay = fs::read_to_string(
        output_dir
            .path()
            .join("packagefiles/ExampleThing-x86_64-linux-gnu/meson.build"),
    )
    .unwrap();
    assert!(linux_overlay.contains(
        "libexample = cc.find_library('example', dirs: meson.current_source_dir() / 'lib')"
    ));
    assert!(linux_overlay.contains("examplething_dep = declare_dependency("));
    // The declared dependency carries the JLL's full release version so a
    // consumer can pin it.
    assert!(linux_overlay.contains("version: '1.2.3+0'"));
    // The whole lib/ runtime directory is installed, not just the declared
    // products, so undeclared transitive runtime libraries come along too.
    assert!(linux_overlay.contains("install_subdir("));
    assert!(linux_overlay.contains("exclude_directories: ['cmake', 'pkgconfig', 'gcc']"));

    // This fixture never splits a platform by ABI, so no options file.
    assert!(!output_dir
        .path()
        .join("packagefiles/ExampleThing/meson.options")
        .exists());

    let error = generate::write_wrap_set(&package, output_dir.path(), false).unwrap_err();
    assert!(matches!(error, Error::AlreadyExists { .. }));

    generate::write_wrap_set(&package, output_dir.path(), true)
        .expect("forced regeneration should succeed");
}
