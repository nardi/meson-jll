# The lockfile format

`meson-jll` records what it has installed in a lockfile at `meson-jll.lock`
in the project root, next to `meson.build`. This page is the formal
specification of that file.

The lockfile is written and read only by `meson-jll`. It should not be
edited by hand: hand edits are not validated against `[compat]` bounds the
way a resolve is, and the next `install` or `update` overwrites the file
based on what it resolves, not on what was edited. It should be committed to
version control, the same as the generated wraps, so that everyone building
the project resolves to the same versions.

## Shape

A complete, worked example:

```toml
version = 1

[roots]
SuiteSparse = "*"

[[package]]
name = "SuiteSparse"
version = "7.12.1+0"
dependencies = ["libblastrampoline"]

[[package]]
name = "libblastrampoline"
version = "5.11.2+2"
dependencies = []
```

## The `version` field

A top-level integer, currently `1`. This is the lockfile *format* version,
not a JLL version. It exists so a future change to this format can be
detected instead of misread: `meson-jll` refuses to read a lockfile whose
`version` it does not recognise, with an error that tells the user to
upgrade the tool. A newer `meson-jll` always stays able to read an older
lockfile version it already shipped support for. A lockfile missing this
field entirely is treated the same as an unrecognised version, since every
lockfile `meson-jll` itself writes always includes it.

## The `[roots]` table

Keys are the bare names (without their `_jll` suffix) of the packages the
user explicitly installed, for example by running `meson-jll install
SuiteSparse`. A package pulled in only because something else depends on it
is not a root.

Values are the pin for that root: the literal string `"*"` if the user did
not request a specific version (so `update` and future installs are free to
move it to a newer one), or a concrete version string (matching one of the
`version` values below) if the user pinned it with a trailing version
argument.

Roots are the entry points a re-resolve starts from: `meson-jll` walks the
dependency graph outward from every name in `[roots]`, not from every
`[[package]]` entry, since a `[[package]]` entry that stopped being reachable
from any root is pruned on the next resolve rather than kept around forever.

## The `[[package]]` array of tables

One entry per package in the resolved dependency graph, roots and their
transitive dependencies alike. Each has:

- `name`: the bare package name, for example `SuiteSparse`.
- `version`: the resolved JLL release version this package is locked to,
  exactly as published, build number included (for example `7.12.1+0`, not
  `7.12.1`). This is also the git tag suffix `meson-jll` fetches the
  package's metadata from when regenerating its wrap set.
- `dependencies`: the bare names of this package's own direct JLL
  dependencies at the locked version. These are the edges `meson-jll` reads
  when deciding which locked packages a later `install` or `update` is
  allowed to move: everything reachable from the package being installed or
  updated through these edges is free to change version, and everything
  else is pinned to what is already here. See `crate::internals`,
  "Resolving versions", for the full explanation of that scheme.

## Guarantees

A reader, whether that is `meson-jll` itself or a human looking at a diff,
can rely on:

- **Sorted.** `[[package]]` entries are always written sorted by `name`, so
  regenerating the lock without changing any version produces a
  byte-identical file, and a real change in versions produces a small,
  readable diff instead of a reshuffled one.
- **Transitively closed.** Every name that appears in some package's
  `dependencies` also has its own `[[package]]` entry. There is no dangling
  edge.
- **Deterministic.** Resolving the same roots against the same pins and the
  same published package versions always produces the same lockfile.
