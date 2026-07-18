using JLLWrappers

export libexample

JLLWrappers.@declare_library_product(libexample, "libexample.1.dylib")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/libexample.dylib",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
