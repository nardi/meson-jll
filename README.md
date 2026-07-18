# `meson-jll`

Julia's ecosystem ships precompiled C, C++, and Fortran binaries for many
scientific software packages as [JLL packages](https://docs.binarybuilder.org/stable/jll).
Each JLL is a GitHub repository that fully describes, for every platform it
supports, the binary tarball to download, the libraries inside it, and any
other JLLs it depends on.

The [Meson](https://mesonbuild.com) build system's [wrap dependency system](https://mesonbuild.com/Wrap-dependency-system-manual.html)
is the standard way to pull an external dependency into a Meson project as a
subproject, so that a plain `dependency('Foo')` call resolves automatically.

`meson-jll` bridges the two. It reads a JLL's metadata and writes a set of
Meson wrap files so that any Meson project can consume Julia-built binaries
with a normal `dependency()` call, on any platform the JLL supports, without
downloading or rebuilding anything by hand. This makes the large collection
of JLL binaries easily usable by projects in any language that the Meson
build system supports, which includes C, C++, C#, D, Fortran, Java, Python
and Rust.

Example usage:

```shell
$ meson-jll install SuiteSparse
```

```python
# meson.build
suitesparse = dependency('SuiteSparse')
executable('demo', 'demo.c', dependencies: suitesparse)
```

For more information, [view the docs here](https://nardi.github.io/meson-jll).
