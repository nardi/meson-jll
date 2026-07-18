# Examples

These examples walk through using a JLL binary from a real Meson project.
Both use [SuiteSparse][suitesparse], a widely used collection of sparse
matrix libraries, because it is a good stand-in for the kind of native
dependency that is otherwise cumbersome to obtain: it is large, it has its
own dependency on a BLAS library, and it ships different binaries for every
platform. `meson-jll` makes all of that a single command.

[suitesparse]: https://github.com/JuliaBinaryWrappers/SuiteSparse_jll.jl

## A C program

Suppose we have a small C program that asks SuiteSparse for its version:

```c
#include <stdio.h>
#include <SuiteSparse_config.h>

int main(void) {
    int version[3];
    SuiteSparse_version(version);
    printf("SuiteSparse %d.%d.%d\n", version[0], version[1], version[2]);
    return 0;
}
```

We want to build it with Meson, so we start with a `meson.build` that
declares SuiteSparse as an ordinary dependency:

```python
project('demo', 'c')
suitesparse = dependency('SuiteSparse')
executable('demo', 'demo.c', dependencies: suitesparse)
```

On its own, that `dependency('SuiteSparse')` call fails, because Meson has
no idea where to find SuiteSparse. We provide it by generating a wrap set
from the JLL:

```shell
$ meson-jll install SuiteSparse
```

This writes a set of wrap files into `subprojects/`, one describing
SuiteSparse and one for every JLL it depends on. Nothing else in the project
changes. The `dependency('SuiteSparse')` call now resolves to the generated
wrap, and a normal Meson build downloads the one binary that matches the
current machine and links against it:

```shell
$ meson setup build
$ meson compile -C build
$ ./build/demo
```

The generated files are plain text that can be committed to version control.
Anyone who checks out the project builds it the same way, on any platform
SuiteSparse supports, with no extra steps.

## A Python extension module

The same wrap works when the thing being built is a Python extension rather
than a plain executable. This is useful with [meson-python][meson-python],
which builds Python packages using Meson, because it lets a compiled
extension link against a heavy native library without the package having to
vendor or build that library itself.

[meson-python]: https://mesonbuild.com/meson-python/

The project layout is a normal meson-python package. The `pyproject.toml`
selects the build backend:

```toml
[build-system]
requires = ["meson-python"]
build-backend = "mesonpy"
```

The `meson.build` builds one extension module and links it against the
SuiteSparse wrap, exactly as the C example did:

```python
project('demo_ext', 'c')
python = import('python').find_installation()
suitesparse = dependency('SuiteSparse')
python.extension_module(
    '_demo',
    '_demo.c',
    dependencies: suitesparse,
    install: true,
)
```

Before building, we generate the wrap set once, the same way as before:

```shell
$ meson-jll install SuiteSparse
```

From then on the package builds and installs with the usual Python tooling,
for example `pip install .`, and the SuiteSparse binary is fetched and
linked as part of that build. The wrap set lives in `subprojects/` and is
committed alongside the rest of the package.
