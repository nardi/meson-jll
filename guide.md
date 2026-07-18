# `meson-jll`

Julia's ecosystem ships precompiled C, C++, and Fortran binaries as JLL
packages, for example [`SuiteSparse_jll.jl`][suitesparse]. Each JLL is a
GitHub repository under `JuliaBinaryWrappers` that fully describes, for every
platform it supports, the binary tarball to download, the libraries inside
it, and any other JLLs it depends on.

Meson's [wrap dependency system][wrap] is the standard way to pull an
external dependency into a Meson project as a subproject, so that a plain
`dependency('Foo')` call resolves automatically.

`meson-jll` bridges the two. It reads a JLL's metadata and writes a set of
Meson wrap files so that any Meson project can consume Julia-built binaries
with a normal `dependency()` call, on any platform the JLL supports, without
downloading or rebuilding anything by hand.

[suitesparse]: https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl
[wrap]: https://mesonbuild.com/Wrap-dependency-system-manual.html

## Installing a dependency

```text
meson-jll install SuiteSparse
```

This writes a wrap set into `subprojects/`, including every JLL package
`SuiteSparse_jll` depends on. Afterwards, a consuming `meson.build` needs
nothing more than:

```meson
suitesparse = dependency('SuiteSparse')
executable('demo', 'demo.c', dependencies: suitesparse)
```

`meson setup` then downloads only the one tarball that matches the machine
it runs on, no matter how many platforms the JLL supports.

## Command overview

The command line mirrors Meson's own `meson wrap`, so the mental model
carries over directly for anyone who has used it before:

- `meson-jll list` lists every JLL package published under
  `JuliaBinaryWrappers`.
- `meson-jll search <term>` searches that list by name.
- `meson-jll install <name> [<version>]` writes a JLL's wrap set into
  `subprojects/`. A specific JLL release can be pinned by version, and
  `--url` reads the package's metadata from somewhere other than the
  `JuliaBinaryWrappers` organisation, for example a private fork.
- `meson-jll info <name>` lists a JLL's available release versions.
- `meson-jll status` lists the JLL wraps already installed in the current
  project and whether newer versions are available.
- `meson-jll update [<name>]` regenerates an installed JLL's wrap set to its
  latest version, or every installed JLL when no name is given.

## How the generated wraps work

A `.wrap` file's `[wrap-file]` section only accepts a single
`source_url`, but a JLL publishes one tarball per platform. `meson-jll`
resolves this by writing a small tree of wraps instead of one:

- A **selector wrap** (`SuiteSparse.wrap`) declares the public dependency
  name through `[provide]`, but carries no source archive of its own. Its
  overlay `meson.build` maps Meson's `host_machine` to the matching
  platform and calls `subproject()` on that platform's own wrap.
- One **binary wrap** per supported platform (for example
  `SuiteSparse-x86_64-linux-gnu.wrap`), each a completely ordinary wrap
  pointing at that platform's tarball. Its overlay turns the extracted files
  into a `declare_dependency()`, using the exact library paths the JLL
  itself declares.

Because Meson only ever fetches a subproject once something actually
references it, only the platform matching the machine running `meson setup`
is ever downloaded. The rest of the wrap set sits untouched in
`subprojects/`, portable across every machine it was generated for.

## A look at the underlying data model

The library crate this binary is built on exposes the pieces above as plain
Rust types, most notably [`meson_jll::jll::triplet::Triplet`], which turns a
JLL's platform selectors into the identifier used to name generated files:

```rust
use meson_jll::jll::triplet::{Arch, Libc, Os, Triplet};

let triplet = Triplet {
    arch: Arch::X86_64,
    os: Os::Linux,
    libc: Some(Libc::Glibc),
    call_abi: None,
    cxxstring_abi: None,
    libgfortran_version: None,
};

assert_eq!(triplet.identifier(), "x86_64-linux-gnu");
```

See the module documentation in the sidebar for the rest of the API:
parsing (`jll`), fetching (`source`, `registry`), and rendering
(`generate`).
