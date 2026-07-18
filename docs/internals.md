# How it works

This page explains what `meson-jll` generates and how it turns a JLL's
metadata into that output. None of it is needed to use the tool, but it
helps when reading the generated files or working on the library.

## The shape of a generated wrap set

A `.wrap` file's `[wrap-file]` section only accepts a single `source_url`,
but a JLL publishes one tarball per platform. `meson-jll` resolves this
mismatch by writing a small tree of wraps instead of one:

- A **selector wrap** (`PackageName.wrap`) declares the public dependency
  name through `[provide]`, but carries no source archive of its own. Its
  overlay `meson.build` maps Meson's `host_machine` to the matching platform
  and calls `subproject()` on that platform's own wrap.
- One **binary wrap** per supported platform (for example
  `PackageName-x86_64-linux-gnu.wrap`), each a completely ordinary wrap
  pointing at that platform's tarball. Its overlay turns the extracted files
  into a `declare_dependency()`, using the exact library paths the JLL
  itself declares.

Because Meson only ever fetches a subproject once something actually
references it, only the platform matching the machine running `meson setup`
is ever downloaded. The rest of the wrap set sits untouched in
`subprojects/`, portable across every machine it was generated for.

## The generation process

Generating a wrap set happens in three steps, each backed by its own part
of the library.

### Resolve where the metadata lives

A package name is turned into something that can fetch files. By default
[`registry::resolve`](crate::registry::resolve) maps a name such as
`SuiteSparse` to the `JuliaBinaryWrappers/SuiteSparse_jll.jl` repository, and
a [`source::GithubSource`](crate::source::GithubSource) reads raw files from
it. A `--url` argument instead produces a
[`source::CustomSource`](crate::source::CustomSource), which reads from a
given repository or local directory. Both implement the one small
[`source::Source`](crate::source::Source) trait, so the rest of the pipeline
does not care which was used.

### Parse the metadata into one model

[`jll::load`](crate::jll::load) reads the three files a JLL publishes and
folds them into a single [`jll::JllPackage`](crate::jll::JllPackage). That
value is a small tree of plain types, one per piece of the metadata:

- [`jll::JllPackage`](crate::jll::JllPackage) is the whole package: its name,
  its version, the names of the other JLLs it depends on, and the list of
  platforms it supports. It comes from `Project.toml` together with the
  entries below.
- [`jll::ResolvedPlatform`](crate::jll::ResolvedPlatform) is one supported
  platform: an [`jll::artifacts::Platform`](crate::jll::artifacts::Platform)
  paired with the libraries found for it. `Artifacts.toml` provides one
  `Platform` per entry, carrying the tarball URL and hash.
- [`jll::triplet::Triplet`](crate::jll::triplet::Triplet) describes a
  platform's architecture, operating system, and ABI. It knows both the
  identifier used to name generated files and the `host_machine` values that
  select it, which is the pairing the selector wrap relies on.
- [`jll::wrappers::LibraryProduct`](crate::jll::wrappers::LibraryProduct) is
  one library inside a platform's tarball, parsed from
  `src/wrappers/<triplet>.jl`. This is the same source of truth Julia itself
  uses.

As an example of what these types capture, a `Triplet` renders to the
identifier that names its generated files:

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

### Render and write the files

[`generate::write_wrap_set`](crate::generate::write_wrap_set) turns one
`JllPackage` into the selector wrap, the per-platform binary wraps, and all
of their overlays. Each file is produced from a compiled template in
`templates/`, fed by a small context struct from
[`generate::context`](crate::generate::context) that has already worked out
every value the template needs, so the templates stay free of logic.

Dependencies on other JLLs are handled by walking the graph:
[`install::install_recursive`](crate::install::install_recursive) generates
a package and then generates each of its JLL dependencies the same way,
skipping any it has already written so a shared dependency is generated only
once.
