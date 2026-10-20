using JLLWrappers

export libexample, libnested

JLLWrappers.@declare_library_product(libexample, "libexample.so.1")
JLLWrappers.@declare_library_product(libnested, "libnested.so.1")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/libexample.so",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
    JLLWrappers.@init_library_product(
        libnested,
        "lib/nested/variant/lib/libnested.so",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
