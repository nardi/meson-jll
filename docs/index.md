# `meson-jll`

Julia's ecosystem ships precompiled C, C++, and Fortran binaries for many
scientific software packages as [JLL packages][jll], for example
[`SuiteSparse_jll.jl`][suitesparse]. Each JLL is a GitHub repository under
[`JuliaBinaryWrappers`][julia-binary-wrappers] that fully describes, for
every platform it supports, the binary tarball to download, the libraries
inside it, and any other JLLs it depends on.

The [Meson][meson] build system's [wrap dependency system][wrap] is the
standard way to pull an external dependency into a Meson project as a
subproject, so that a plain `dependency('Foo')` call resolves automatically.

`meson-jll` bridges the two. It reads a JLL's metadata and writes a set of
Meson wrap files so that any Meson project can consume Julia-built binaries
with a normal `dependency()` call, on any platform the JLL supports, without
downloading or rebuilding anything by hand. This makes the large collection
of JLL binaries easily usable by projects in any language that the Meson
build system supports, which includes C, C++, C#, D, Fortran, Java, Python
and Rust.

For worked examples of using a JLL from a real project, see the
[examples] page. For an explanation of what the generated
files are and how they are produced, see the [internals]
page.

[jll]: https://docs.binarybuilder.org/stable/jll/
[julia-binary-wrappers]: https://github.com/orgs/JuliaBinaryWrappers/repositories
[meson]: https://mesonbuild.com
[suitesparse]: https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl
[wrap]: https://mesonbuild.com/Wrap-dependency-system-manual.html

## Installation

Every [release](https://github.com/nardi/meson-jll/releases) includes a
prebuilt `meson-jll` archive for each supported platform. Download the one
for your platform and put the binary on `PATH`. To build from source
instead, run:

```shell
$ cargo install --locked --git https://github.com/nardi/meson-jll
```

A project that wants no installation step at all can let Meson fetch the
binary itself, as described under "Version control" below.

## Getting started

Generate a wrap set for a JLL, then use it like any other Meson dependency:

```shell
$ meson-jll install SuiteSparse
```
```python
# meson.build
suitesparse = dependency('SuiteSparse_jll')
executable('demo', 'demo.c', dependencies: suitesparse)
```

`meson setup` then downloads only the one tarball that matches the machine
it runs on, no matter how many platforms the JLL supports. The
[examples] page walks through this end to end.

## Commands

The command line mirrors Meson's own `meson wrap`, so the mental model
carries over directly for anyone who has used it before.

### `list` and `search`

`list` prints every JLL package published under `JuliaBinaryWrappers`, and
`search` filters that list by name:

```shell
$ meson-jll search suitesparse
SuiteSparse
```

### `install`

`install <name> [<version>]` writes a JLL's wrap set into `subprojects/`,
including a wrap for every JLL it depends on. It prints each package it
wrote, with the version it resolved:

```shell
$ meson-jll install SuiteSparse
installed SuiteSparse 7.12.1+0
installed libblastrampoline 5.11.2+2
```

A trailing version pins a specific JLL release instead of the latest one.
The `--url` option reads the package's metadata from somewhere other than
the `JuliaBinaryWrappers` organization, for example a private fork, and
`--force` overwrites wrap files that already exist.

### `info`

`info <name>` lists the release versions available for a JLL, newest first:

```shell
$ meson-jll info SuiteSparse
7.12.1+0
7.11.0+0
7.10.1+0
...
```

### `status`

`status` lists the JLL wraps already installed in the current project and
whether a newer version exists:

```shell
$ meson-jll status
SuiteSparse 7.12.1+0 (up to date)
libblastrampoline 5.11.2+2 (latest: 5.12.0+0)
```

### `update`

`update [<name>]` regenerates an installed JLL's wrap set at its latest
version, or every installed JLL when no name is given:

```shell
$ meson-jll update SuiteSparse
updated SuiteSparse to 7.13.0+0
```

### `sync`

`sync` regenerates every wrap straight from the committed lockfile, without
resolving anything or contacting the registry for versions. It is the
command to run after a fresh checkout of a project that commits its
`meson-jll.lock.wrap` but not the generated wraps (see "Version control"
below):

```shell
$ meson-jll sync
Synced SuiteSparse 7.12.1+0
Synced libblastrampoline 5.11.2+2
```

## Version control

`meson-jll` writes everything under `subprojects/`. That includes the
lockfile `meson-jll.lock.wrap`, and the wraps and `packagefiles/` overlays
generated for each JLL. On top of those, `meson setup` later adds more files
under `subprojects/` of its own (downloaded archives and the source trees it
extracts from them). There are two suggested approaches to include this in
version control.

In either case, you should always commit `meson-jll.lock.wrap`. It is the
record of exactly which JLL versions the project resolved to, and everything
else can be regenerated from it.

A project from an earlier version has its lock in `meson-jll.lock` in the
project root instead. The commands read that file until the next `install`
or `update` moves the lock into `meson-jll.lock.wrap` and deletes it, so
commit that change.

**Approach one: commit the lockfile only.** Ignore everything in
`subprojects/` except the lockfile. A collaborator, or a CI job, runs
`meson-jll sync` after checking out, which regenerates every wrap from the
lockfile before building. This keeps the repository small and free of
generated files, at the cost of one extra command before the first build.

```gitignore
# Commit the lockfile, regenerate the rest.
subprojects/*
!subprojects/meson-jll.lock.wrap
```

The extra command can be folded into `meson setup` itself, so a collaborator
never has to remember it. Look up `meson-jll` with `find_program()` and call
`sync` from a `run_command()` at the top of the root `meson.build`, before
anything reads from `subprojects/`.

```meson
meson_jll = find_program('meson-jll')
run_command(meson_jll, 'sync', check: true)
```

This makes `meson setup` self-sufficient straight after a fresh checkout.
`find_program()` uses the `meson-jll` on `PATH` when there is one. When
there is not, it falls back to a prebuilt binary that Meson downloads
through the wrap sections of `meson-jll.lock.wrap`, so the project
configures on a machine that never installed the tool. Without those
sections, a machine that lacks `meson-jll` stops at this line with a message
naming the missing program.

The first `meson-jll install` in a project sets those sections up. It starts
`meson-jll.lock.wrap` from the `meson-jll.lock.wrap` file attached to the
GitHub release of the version that is running, and writes the lock below it.
A later `install` or `update` keeps them. The `MESON_JLL_WRAP` environment
variable names a URL or a path to read them from instead, for example a
mirror. If they cannot be read, because the build is not a release or there
is no network, the file starts with a placeholder wrap section, and the
project has no fallback until the sections are added.

The sections name the version of the release they came from, and that is the
`meson-jll` version Meson downloads. To upgrade, replace the lines above the
first `[meson-jll]` section with the lines of a newer release's file.
Releases include binaries for Linux (x86_64 and aarch64, statically linked
so that any distribution works), macOS (x86_64 and aarch64), and Windows
(x86_64). On any other platform `find_program()` fails with an error that
names the platform. See [internals] for how the wrap is
put together.

**Approach two: commit the generated wraps too.** Commit the `.wrap` files
and the `packagefiles/` overlay directory `meson-jll` generates, so the
project builds straight after checkout with no `sync` step. Ignore only the
files `meson setup` adds on its own: the download cache and the extracted
source trees.

```gitignore
# Ignore only what meson itself downloads and extracts.
subprojects/packagecache/
subprojects/*/
!subprojects/packagefiles/
```

The `subprojects/*/` line ignores every extracted source directory (each
platform's tarball is unpacked into its own directory there), while the
following line keeps the `packagefiles/` overlays `meson-jll` generated.
The individual `.wrap` files are not directories, so they are committed
either way, `meson-jll.lock.wrap` included.

## Common issues

Some JLLs need help from the project that links them, which the generated
wraps cannot provide.

### BLAS libraries

`libblastrampoline` has no BLAS backend of its own. In Julia, the `__init__`
function of the package forwards OpenBLAS to it when the package loads. A
project that links the library outside of Julia has no such step, so the
first BLAS call crashes with a segmentation fault. This happens, for
example, when `Ipopt_jll` is called from `SCIP_jll`.

To avoid the crash, forward OpenBLAS once before anything else uses BLAS. A
consumer that loads the libraries from Python can call `lbt_forward` on
`libopenblas.so` once at import.

## Where to next

- [Examples](crate::examples): a worked C program and a meson-python
  extension module, both consuming a JLL end to end.
- [Internals](crate::internals): how the generated wrap set is shaped, the
  three-step generation process, and how versions are resolved.
- [Lockfile format](crate::lockfile): the formal specification of
  `meson-jll.lock.wrap`.
