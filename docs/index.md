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

## Where to next

- [Examples](crate::examples): a worked C program and a meson-python
  extension module, both consuming a JLL end to end.
- [Internals](crate::internals): how the generated wrap set is shaped, the
  three-step generation process, and how versions are resolved.
- [Lockfile format](crate::lockfile): the formal specification of
  `subprojects/meson-jll.lock`.
