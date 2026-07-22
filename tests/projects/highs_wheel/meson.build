project('demo_ext', 'c')
python = import('python').find_installation(pure: false)
highs = dependency('HiGHS_jll')
python.extension_module(
    '_demo',
    '_demo.c',
    dependencies: highs,
    install: true,
    subdir: 'demo_ext',
)
python.install_sources('demo_ext/__init__.py', subdir: 'demo_ext')
