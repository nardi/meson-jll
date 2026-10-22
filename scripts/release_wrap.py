"""Builds the release assets that let Meson fetch `meson-jll` itself.

A project that commits `subprojects/meson-jll.lock.wrap` can call
`find_program('meson-jll')` on a machine without the tool, because the wrap
declares `meson-jll` as a program it provides. A `.wrap` file can only name
one source archive, but there is one release binary per platform, so the
wrap points at a small archive of wraps instead. That archive holds one
wrap per platform and a `meson.build` that picks the one matching the
machine running Meson. Only the matching binary is ever downloaded.

The `package` subcommand turns a built binary into a release archive. The
`wrap` subcommand takes those archives and writes the wrap archive and the
`meson-jll.lock.wrap` file that refers to it. See `crate::lockfile` for how
the lock shares that file.
"""

import argparse
import gzip
import hashlib
import io
import tarfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Target:
    """A platform a release binary is built for."""

    triple: str
    """The Rust target triple, which also names the release archive."""

    system: str
    """The value of Meson's `build_machine.system()` on this platform."""

    cpu_family: str
    """The value of Meson's `build_machine.cpu_family()` on this platform."""

    @property
    def is_windows(self) -> bool:
        return self.system == "windows"

    @property
    def archive_extension(self) -> str:
        return "zip" if self.is_windows else "tar.gz"

    @property
    def archive_name(self) -> str:
        """The file name of the release archive holding the binary."""
        return f"meson-jll-{self.triple}.{self.archive_extension}"

    @property
    def binary_name(self) -> str:
        """The file name of the binary inside the release archive."""
        return "meson-jll.exe" if self.is_windows else "meson-jll"


# Linux binaries are statically linked against musl, so one binary runs on
# any distribution and the wrap does not have to tell glibc and musl apart.
TARGETS = (
    Target("x86_64-unknown-linux-musl", "linux", "x86_64"),
    Target("aarch64-unknown-linux-musl", "linux", "aarch64"),
    Target("x86_64-apple-darwin", "darwin", "x86_64"),
    Target("aarch64-apple-darwin", "darwin", "aarch64"),
    Target("x86_64-pc-windows-msvc", "windows", "x86_64"),
)

WRAP_ARCHIVE_NAME = "meson-jll-wrap.tar.gz"
LOCK_WRAP_NAME = "meson-jll.lock.wrap"


def find_target(triple: str) -> Target:
    """Looks a target up by its triple, or exits with the valid choices."""
    for target in TARGETS:
        if target.triple == triple:
            return target
    choices = ", ".join(target.triple for target in TARGETS)
    raise SystemExit(f"Unknown target {triple}. The targets are: {choices}.")


def sha256_of(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package_binary(target: Target, binary: Path, output_directory: Path) -> Path:
    """Writes the release archive for `target`, holding only the binary."""
    output_directory.mkdir(parents=True, exist_ok=True)
    archive_path = output_directory / target.archive_name
    if target.is_windows:
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.write(binary, target.binary_name)
    else:
        with tarfile.open(archive_path, "w:gz") as archive:
            # The binary keeps its mode, so the executable bit survives.
            archive.add(binary, arcname=target.binary_name)
    return archive_path


def binary_wrap(target: Target, version: str, repository: str, archive: Path) -> str:
    """The wrap for one platform's binary, inside the wrap archive."""
    # The cache file name carries the version, so a cached archive of an
    # older release is never mistaken for this one.
    cache_name = f"meson-jll-{version}-{target.triple}.{target.archive_extension}"
    url = f"https://github.com/{repository}/releases/download/v{version}/{target.archive_name}"
    return f"""\
[wrap-file]
directory = meson-jll-{target.triple}
source_url = {url}
source_filename = {cache_name}
source_hash = {sha256_of(archive)}
lead_directory_missing = true
patch_directory = meson-jll-{target.triple}
"""


def binary_overlay(target: Target) -> str:
    """The `meson.build` that makes a platform's binary the `meson-jll` program."""
    return f"""\
project('meson-jll-{target.triple}')

# Every other `find_program('meson-jll')` call in the build resolves to the
# binary this archive was extracted with.
meson.override_find_program('meson-jll', find_program(meson.current_source_dir() / '{target.binary_name}'))
"""


def selector_build_file(targets: tuple[Target, ...]) -> str:
    """The `meson.build` that picks the wrap for the machine running Meson."""
    branches = []
    for index, target in enumerate(targets):
        keyword = "if" if index == 0 else "elif"
        branches.append(
            f"{keyword} system == '{target.system}' and cpu_family == '{target.cpu_family}'\n"
            f"  triple = '{target.triple}'"
        )
    chain = "\n".join(branches)
    return f"""\
project('meson-jll-wrap')

# The tool runs while Meson configures the project, so the machine that has
# to run it is the build machine, even when the project is cross compiled.
system = build_machine.system()
cpu_family = build_machine.cpu_family()

triple = ''
{chain}
else
  error('meson-jll: there is no prebuilt binary for ' + system + ' on ' + cpu_family + '.')
endif

# Meson only fetches a subproject once something references it, so only the
# binary for the matching platform is downloaded.
subproject('meson-jll-' + triple)
"""


def write_wrap_archive(
    version: str,
    repository: str,
    archives_directory: Path,
    targets: tuple[Target, ...],
    archive_path: Path,
) -> None:
    """Packs the selector and the per-platform wraps into one archive.

    The archive is reproducible, so building it twice from the same inputs
    gives the same hash, which the `meson-jll.lock.wrap` file records.
    """
    root = f"meson-jll-wrap-{version}"
    files: dict[str, str] = {f"{root}/meson.build": selector_build_file(targets)}
    for target in targets:
        archive = archives_directory / target.archive_name
        files[f"{root}/subprojects/meson-jll-{target.triple}.wrap"] = binary_wrap(
            target, version, repository, archive
        )
        overlay = f"{root}/subprojects/packagefiles/meson-jll-{target.triple}/meson.build"
        files[overlay] = binary_overlay(target)

    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name in sorted(files):
            content = files[name].encode()
            entry = tarfile.TarInfo(name)
            entry.size = len(content)
            entry.mode = 0o644
            archive.addfile(entry, io.BytesIO(content))
    with archive_path.open("wb") as raw_file:
        with gzip.GzipFile(fileobj=raw_file, mode="wb", mtime=0) as compressed:
            compressed.write(buffer.getvalue())


def lock_wrap(version: str, repository: str, wrap_archive: Path) -> str:
    """The wrap file a project commits as `subprojects/meson-jll.lock.wrap`."""
    url = f"https://github.com/{repository}/releases/download/v{version}/{WRAP_ARCHIVE_NAME}"
    return f"""\
# Lets Meson fetch meson-jll {version} when it is not on PATH. The sections
# named meson-jll below it hold the lock, which meson-jll writes itself.
[wrap-file]
directory = meson-jll-wrap-{version}
source_url = {url}
source_filename = meson-jll-wrap-{version}.tar.gz
source_hash = {sha256_of(wrap_archive)}

[provide]
program_names = meson-jll
"""


def run_package(arguments: argparse.Namespace) -> None:
    target = find_target(arguments.target)
    archive_path = package_binary(target, arguments.binary, arguments.output)
    print(archive_path)


def run_wrap(arguments: argparse.Namespace) -> None:
    triples = arguments.targets or [target.triple for target in TARGETS]
    targets = tuple(find_target(triple) for triple in triples)
    arguments.output.mkdir(parents=True, exist_ok=True)

    wrap_archive = arguments.output / WRAP_ARCHIVE_NAME
    write_wrap_archive(
        arguments.version, arguments.repository, arguments.archives, targets, wrap_archive
    )
    lock_wrap_path = arguments.output / LOCK_WRAP_NAME
    lock_wrap_path.write_text(lock_wrap(arguments.version, arguments.repository, wrap_archive))
    print(wrap_archive)
    print(lock_wrap_path)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    subcommands = parser.add_subparsers(dest="command", required=True)

    package = subcommands.add_parser("package", help="Archive a built binary.")
    package.add_argument("--target", required=True, help="The Rust target triple.")
    package.add_argument("--binary", required=True, type=Path, help="The built binary.")
    package.add_argument("--output", required=True, type=Path, help="Where to write the archive.")
    package.set_defaults(run=run_package)

    wrap = subcommands.add_parser("wrap", help="Write the wrap archive and lock wrap.")
    wrap.add_argument("--version", required=True, help="The release version, such as 0.1.0.")
    wrap.add_argument("--repository", required=True, help="The GitHub repository, as owner/name.")
    wrap.add_argument("--archives", required=True, type=Path, help="Where the release archives are.")
    wrap.add_argument("--output", required=True, type=Path, help="Where to write the results.")
    wrap.add_argument(
        "--targets",
        nargs="+",
        help="Only include these target triples. Every target is included by default.",
    )
    wrap.set_defaults(run=run_wrap)

    arguments = parser.parse_args()
    arguments.run(arguments)


if __name__ == "__main__":
    main()
