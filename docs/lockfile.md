# The lockfile format

`meson-jll` records what it has installed in a lockfile at
`subprojects/meson-jll.lock.wrap`. This page is the formal specification of
that file.

The lock is stored in a Meson wrap file on purpose. Meson has to read a
committed file before `meson-jll` itself can run, so the lock can share a
file with a wrap that fetches `meson-jll`. Meson ignores the sections of a
wrap file that it does not know. `meson-jll` only reads and writes the
sections named `meson-jll` or starting with `meson-jll.`, and it leaves
every other line of the file as it found it.

The lockfile is written and read only by `meson-jll`. It should not be
edited by hand, because hand edits are not validated against `[compat]`
bounds the way a resolve is, and the next `install` or `update` overwrites
the lock sections based on what it resolves, not on what was edited. It
should be committed to version control, so that everyone building the
project resolves to the same versions.

## Shape

A complete, worked example:

```ini
# Meson rejects a wrap file that does not start with a wrap section.
# The sections named meson-jll below hold the lock.
[wrap-file]
directory = meson-jll

[meson-jll]
format = 1

[meson-jll.roots]
SuiteSparse = *

[meson-jll.package.SuiteSparse]
version = 7.12.1+0
dependencies = libblastrampoline

[meson-jll.package.libblastrampoline]
version = 5.11.2+2
```

Meson requires the first section of every wrap file to be a wrap section,
even when nothing refers to that file. A new lockfile therefore starts with
the placeholder `[wrap-file]` section shown above. A lockfile that shares
its file with a real wrap keeps that wrap's sections instead. Everything
outside the lock sections is outside this specification.

## The `[meson-jll]` section

Holds one key, `format`, an integer that is currently `1`. This is the
lockfile *format* version, not a JLL version. It exists so a future change
to this format can be detected instead of misread. `meson-jll` refuses to
read a lockfile whose `format` it does not recognise, with an error that
tells the user to upgrade the tool. A newer `meson-jll` always stays able to
read an older format it already shipped support for. A file that has lock
sections but no `format` is treated the same as an unrecognised format,
since every lockfile `meson-jll` itself writes always includes it. A file
with no lock sections at all is an empty lock.

## The `[meson-jll.roots]` section

Keys are the bare names (without their `_jll` suffix) of the packages the
user explicitly installed, for example by running `meson-jll install
SuiteSparse`. A package pulled in only because something else depends on it
is not a root. The section is left out when there are no roots.

Values are the pin for that root: the literal `*` if the user did not
request a specific version (so `update` and future installs are free to move
it to a newer one), or a concrete version string (matching one of the
`version` values below) if the user pinned it with a trailing version
argument.

Roots are the entry points a re-resolve starts from. `meson-jll` walks the
dependency graph outward from every name in this section, not from every
package section, since a package section that stopped being reachable from
any root is pruned on the next resolve rather than kept around forever.

## The `[meson-jll.package.<name>]` sections

One section per package in the resolved dependency graph, roots and their
transitive dependencies alike. The part of the section name after
`meson-jll.package.` is the bare package name, for example `SuiteSparse`.
Each section has:

- `version`: the resolved JLL release version this package is locked to,
  exactly as published, build number included (for example `7.12.1+0`, not
  `7.12.1`). This is also the git tag suffix `meson-jll` fetches the
  package's metadata from when regenerating its wrap set.

- `dependencies`: the bare names of this package's own direct JLL
  dependencies at the locked version, separated by commas. The key is left
  out when there are none. These are the edges `meson-jll` reads when
  deciding which locked packages a later `install` or `update` is allowed to
  move. Everything reachable from the package being installed or updated
  through these edges is free to change version, and everything else is
  pinned to what is already here. See `crate::internals`, "Resolving
  versions", for the full explanation of that scheme.

## Guarantees

A reader, whether that is `meson-jll` itself or a human looking at a diff,
can rely on:

- **Sorted.** Package sections are always written sorted by name, so
  regenerating the lock without changing any version produces a
  byte-identical file, and a real change in versions produces a small,
  readable diff instead of a reshuffled one.

- **Transitively closed.** Every name that appears in some package's
  `dependencies` also has its own package section. There is no dangling
  edge.

- **Deterministic.** Resolving the same roots against the same pins and the
  same published package versions always produces the same lockfile.

- **Contained.** Writing the lock replaces the lock sections and leaves
  every other line of the file unchanged.
