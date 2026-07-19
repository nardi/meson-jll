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
[examples](crate::examples) page. For an explanation of what the generated
files are and how they are produced, see the [internals](crate::internals)
page.

[jll]: https://docs.binarybuilder.org/stable/jll/
[julia-binary-wrappers]: https://github.com/orgs/JuliaBinaryWrappers/repositories
[meson]: https://mesonbuild.com
[suitesparse]: https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl
[wrap]: https://mesonbuild.com/Wrap-dependency-system-manual.html

## Getting started

Generate a wrap set for a JLL, then use it like any other Meson dependency:

```shell
$ meson-jll install SuiteSparse
```
```python
# meson.build
suitesparse = dependency('SuiteSparse')
executable('demo', 'demo.c', dependencies: suitesparse)
```

`meson setup` then downloads only the one tarball that matches the machine
it runs on, no matter how many platforms the JLL supports. The
[examples](crate::examples) page walks through this end to end.

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
`meson-jll.lock` but not the generated wraps (see "Version control" below):

```shell
$ meson-jll sync
Synced SuiteSparse 7.12.1+0
Synced libblastrampoline 5.11.2+2
```

## Version control

`meson-jll` writes into two places: `meson-jll.lock` in the project root,
and a set of files under `subprojects/`. On top of those, `meson setup`
later adds more files under `subprojects/` of its own (downloaded archives
and the source trees it extracts from them). There are two suggested
approaches to include this in version control.

In either case, you should always commit `meson-jll.lock`. It is the record
of exactly which JLL versions the project resolved to, and everything else
can be regenerated from it.

**Approach one: commit the lockfile only.** Ignore the whole `subprojects/`
directory. A collaborator, or a CI job, runs `meson-jll sync` after checking
out, which regenerates every wrap from the lockfile before building. This
keeps the repository small and free of generated files, at the cost of one
extra command before the first build.

```gitignore
# Commit meson-jll.lock (in the project root), regenerate the rest.
subprojects/
```

The extra command can be folded into `meson setup` itself, so a collaborator
never has to remember it: call `meson-jll sync` from a `run_command()` at the
top of the root `meson.build`, before anything reads from `subprojects/`.

```meson
run_command('meson-jll', 'sync', check: true)
```

This makes `meson setup` self-sufficient straight after a fresh checkout, at
the cost of requiring `meson-jll` itself to be on `PATH` wherever the project
is configured, including CI.

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
either way.

## Where to next

- [Examples](crate::examples): a worked C program and a meson-python
  extension module, both consuming a JLL end to end.
- [Internals](crate::internals): how the generated wrap set is shaped, the
  three-step generation process, and how versions are resolved.
- [Lockfile format](crate::lockfile): the formal specification of
  `meson-jll.lock`.
