"""Demo package linking against HiGHS from a JLL."""

import os


def _append_to_sharedlib_load_path():
    """Ensure the shared libraries in this package can be loaded on Windows.

    Windows lacks a concept equivalent to RPATH, so the directory that
    meson-python folds the JLL shared libraries into
    (``.demo_ext.mesonpy.libs`` in site-packages) has to be added to the
    DLL search path by the package itself. This is the pattern the
    meson-python documentation prescribes for internal shared libraries
    on Windows.
    """
    if os.name == 'nt':
        libs_dir = os.path.join(
            os.path.dirname(os.path.dirname(__file__)), '.demo_ext.mesonpy.libs'
        )
        if os.path.isdir(libs_dir):
            os.add_dll_directory(libs_dir)


_append_to_sharedlib_load_path()

from ._demo import create_and_destroy, version  # noqa: E402

__all__ = ['create_and_destroy', 'version']
