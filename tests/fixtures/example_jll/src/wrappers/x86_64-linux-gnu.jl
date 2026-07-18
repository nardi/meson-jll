using JLLWrappers

export libexample

JLLWrappers.@declare_library_product(libexample, "libexample.so.1")

function __init__()
    JLLWrappers.@init_library_product(
        libexample,
        "lib/libexample.so",
        RTLD_LAZY | RTLD_DEEPBIND,
    )
end
