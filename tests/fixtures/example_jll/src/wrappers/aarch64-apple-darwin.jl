using JLLWrappers

export libexample, libversioned

JLLWrappers.@declare_library_product(libexample, "libexample.1.dylib")
JLLWrappers.@declare_library_product(libversioned, "libversioned.1.1.dylib")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/libexample.dylib",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
    # No unversioned libversioned.dylib symlink ships in this tarball, the
    # same as libgcc_s in the real CompilerSupportLibraries_jll: the
    # generated find_library() fallback must link straight against this
    # exact versioned path, not a reconstructed plain name.
    JLLWrappers.@init_library_product(
        libversioned,
        "lib/libversioned.1.1.dylib",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
