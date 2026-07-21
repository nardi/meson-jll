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

## The public dependency name

The name a selector wrap provides is the package name with a `_jll` suffix,
for example `dependency('Zlib_jll')`, not a bare `dependency('Zlib')`. This
matters because a JLL bundles its own copy of a library that a build machine
very often already has a system copy of. A bare `dependency('Zlib')` would be
satisfied by that system copy before the wrap ever ran, and the JLL's own
binary, the one meant to end up in the wheel, would silently never build. No
system package advertises itself under a `_jll` name, so the suffixed name can
only ever resolve to the wrap. The same suffixed name is used both for the
public dependency a consumer asks for and for the edges between JLLs
internally. Each `declare_dependency()` also carries the JLL's release version
as its `version:`, so a consumer can pin it, for example
`dependency('HiGHS_jll', version: '>=1.15.0')`.

## Installing the runtime libraries

A binary wrap's overlay installs the platform's runtime libraries into
`libdir`, which is where meson-python folds a wheel's bundled shared libraries
from. It installs the whole runtime directory, `bin/` on Windows and `lib/` on
the rest (minus the build-time `cmake/`, `pkgconfig/`, and `gcc/` trees), not
only the libraries the JLL declares as products. A tarball also ships the
transitive runtime libraries its products depend on, such as libquadmath behind
libgfortran, which are never declared as products of their own and so would
otherwise be missing at load time. Installing the files as they sit in the
tarball also keeps the versioned name the loader actually records as a
dependency (`libhighs.so.1`, not the unversioned dev link `libhighs.so`), which
is the name a wheel needs and the one repair tools like `auditwheel` and
`delocate` look for.

## Stripping the runtime libraries

JLL binaries ship unstripped, with a full symbol table. A bundled
`libstdc++` commonly carries ten times its stripped size in debug and symbol
information. Meson's own `-Dstrip` does not help here on its own: it only
strips targets Meson itself compiled, never a file `install_subdir` copied in
verbatim the way every library above is installed.

The natural fix, an `add_install_script()` that strips whatever was just
installed, does not work for a Python wheel: meson-python never actually
runs `meson install`. It builds a wheel by reading Meson's own static
install plan (`meson introspect --install-plan`, computed entirely at
configure time) and copying those files straight out of the build tree.
An install script produces no such advance listing, since Meson cannot know
ahead of time what an arbitrary script will do, so it is invisible to that
plan and silently never runs for a wheel build, `-Dstrip=true` or not.

Instead, each declared library product gets its own `custom_target()`,
built at compile time (which meson-python does run) rather than install
time, gated on `-Dstrip` and a strip tool (`strip` or `llvm-strip`) being
found. It strips a copy of the product under its own final name, and the
bulk directory install above excludes that one file, so the stripped
`custom_target` output is what actually ships. This is silently skipped,
the same as the MSVC import-lib workaround above, when no strip tool is
found, so it is always safe to leave enabled. It can only reach the
products a triplet overlay already knows how to name, so an undeclared
transitive library (libquadmath, again) ships unstripped regardless.

The actual stripping goes through `strip_or_copy.py` (shared the same way
`dll_to_lib.py` is), not a direct call to the strip tool, because a strip
tool is not always able to parse every binary it is pointed at.
`llvm-strip` in particular has been observed to reject some MinGW-built
COFF binaries outright ("invalid SymbolTableIndex") that it can still link
against fine. Since `-Dstrip` is a size optimization, never a correctness
requirement, `strip_or_copy.py` falls back to a plain copy on failure
rather than taking the whole build down over a library that was always
going to ship unstripped anyway.

On Windows, the same runtime install also always excludes a JLL's own
executable products (`highs.exe` alongside `libhighs.dll`, for example,
parsed the same way library products are, from `@declare_executable_product`
in the wrapper script): a library consumer never needs the JLL's own CLI
tool, and bundling it anyway only bloats a wheel for no reason.

One rough edge remains on macOS, in meson-python rather than here. When an
extension links libraries from several JLL subprojects, each contributes its
own `@loader_path`-relative `LC_RPATH`, and meson-python rewrites all of them to
the same bundled-libraries path, producing duplicate load commands that recent
dyld rejects. Until meson-python de-duplicates after rewriting, a consumer that
hits this can strip the duplicates as a post-repair step.

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

Dependencies on other JLLs are not generated one at a time as they are
discovered. They go through the version resolver first, covered next, which
decides one consistent version for every package in the graph before
anything is written.

## Resolving versions

A JLL declares version bounds on its own JLL dependencies in a `[compat]`
section of `Project.toml`, the same way any Julia package does. For example
`SuiteSparse_jll` declares `libblastrampoline_jll = "5.8.0"`, meaning it
needs at least that version. Simply always taking the latest available
version of every dependency, ignoring `[compat]` entirely, mostly works only
because these bounds are almost always floors and latest almost always
clears one. It silently breaks on an upper bound, or when latest drifts
below a declared floor.

`meson-jll` avoids that by resolving the whole dependency graph to one
mutually compatible set of versions before generating anything, the same
way Julia's own `Pkg` resolver keeps an environment consistent, and by
recording the result in a lockfile so the same versions are used again next
time. This section covers the algorithm, [`crate::lockfile`] specifies the
file it is recorded in.

### The fixed-point solver

[`resolve::resolve`](crate::resolve::resolve) is a fixed-point computation,
not a backtracking or SAT solver. It repeatedly resolves each package to the
highest available version satisfying every `[compat]` range accumulated
against it so far from everything that depends on it, and repeats until a
full pass changes nothing. Constraints only ever accumulate over a resolve
and are never retracted: the result can be slightly more conservative than
strictly necessary, since a constraint from a branch that later turns out
irrelevant still applies, but it is never wrong, because a version
satisfying a superset of the real constraints always satisfies the real ones
too.

This is enough for JLL dependency graphs specifically because they are
shallow and generated mechanically from a single upstream build, so
genuinely conflicting compat ranges are rare, unlike the deep, independently
authored graphs a general-purpose package manager has to solve for. A
resolve that does not settle within its iteration budget raises an error
rather than guessing, so a real conflict is always reported rather than
silently papered over.

The one seam to the network is
[`resolve::Catalog`](crate::resolve::Catalog), a trait with two methods,
"what versions exist" and "what does this version depend on". Its real
implementation, [`resolve::GithubCatalog`](crate::resolve::GithubCatalog),
answers both from GitHub: release tags for the first, and each tag's
`Project.toml` for the second. Kept behind a trait and used through a
generic parameter rather than a trait object, matching
[`source::Source`](crate::source::Source), so the solver itself is unit
tested against an in-memory catalog with no network involved.

As a small illustration of the compat parsing this relies on, a bare version
in a compat specifier is a caret range: it accepts anything from that
version up to, but excluding, the next version that would change its
leftmost nonzero component.

```rust
use meson_jll::version::{CompatSpecifier, Version};

let specifier = CompatSpecifier::parse("5.8.0");
assert!(specifier.contains(Version::parse("5.9.0").unwrap()));
assert!(!specifier.contains(Version::parse("6.0.0").unwrap()));
```

### Updating, or installing specific versions

The solver itself is stateless: given the same `required` names, `pins`, and
catalog, it always resolves to the same versions. The behavior that makes
installing or updating one package leave every unrelated package exactly
where it was locked lives one level up, in
[`install::install`](crate::install::install), which builds `pins` from the
project's existing lockfile before ever calling the solver:

- Every locked package **outside** the dependency closure (in the old lock)
  of the package being installed or updated is pinned to its current locked
  version, so it cannot move.
- Everything **inside** that closure is left free, so it can rise if the
  refreshed package's new requirements need a higher version of it.

`update <name>` is exactly `install <name>` with no version pinned, since
installing with no version already means "take the latest available". A
bare `update`, with no name, refreshes every root at once with no pins at
all. See [`crate::lockfile`] for the `[[package]]` `dependencies` edges this
closure is computed from.
