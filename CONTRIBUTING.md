# Contributing to `meson-jll`

This document describes the conventions this project is written and
maintained under, in order to keep to a consistent style and structure.

## Writing style

- Use descriptive type and variable names.
- Comment liberally, in plain language, but be concise. Cross-reference other
  comments if needed.
- Lead with the motivation. Explain why something exists or works the way it
  does before describing what it does.
- If you use an AI agent, please instruct it to mimic the style of existing code
  and documentation. Especially avoid unnatural AI cliches such as overuse of
  em-dashes and semicolons, usage of words like "drives", phrases like "A is the
  central B", "the key distinction is", etc.

## Code

- Prefer borrowing and stack values over heap allocation. Avoid dynamic dispatch
  (`Box<dyn Trait>`), reach for enums and generics first. The few traits in the
  codebase (`Source`, `Catalog`) are still resolved statically through generics,
  never as a trait object.
- Avoid (writing) macros unless they remove a large amount of repetition.
  `serde` derive, `clap` derive, `thiserror`, and `askama`'s `Template` derive
  are the accepted exceptions, each already justified by what they replace.
- Keep modules small and focused. A module's doc comment explains why it
  exists and why it is shaped the way it is, not just what is in it.
- Document every public item with a rustdoc comment.
- Don't add error handling, fallbacks, or abstractions for cases that can't
  happen. Prefer changing your data types so invalid states are not representable.
- Keep it simple, don't make micro-optimizations (like minimizing temporary
  allocations in an I/O bound function) until you have confirmed they are
  significant.

## Documentation

- The user guide lives in `docs/` and is included directly into the crate
  root and its doc-only submodules (`examples`, `internals`, `lockfile`) via
  `#![doc = include_str!(...)]`, so `cargo doc` always serves the current
  guide, never a stale copy.
- New doc modules should be mentioned in `index.md` so they are easily noticed.
- Code snippets in the guide that are meant to run (Rust ones) should stay
  valid under `cargo test`, since they run as doctests. Shell and Meson
  snippets are illustrative and are not executed.
- Run `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` before committing a
  documentation change. A broken intra-doc link (for example one pointing at
  an item that was since made private or renamed) fails this, not a plain
  `cargo doc`.

## Testing

- Parser and logic unit tests run against vendored fixtures or in-memory
  fakes (an in-memory `Catalog` for the resolver, a fixture tree for the
  registry's package-name parsing), never against the network.
- `tests/generate.rs` is the integration test: it runs the generator over a
  fixture JLL and snapshots every file it writes, so a change to the output
  is visible in review.
- `tests/e2e_meson.rs` is the real thing: it builds the `meson-jll` binary,
  generates a wrap set from it, and actually runs `meson setup` and
  `meson compile` against it. It is marked `#[ignore]` because it needs Meson,
  Ninja, and a compiler on `PATH`, and runs in CI (`ci.yml`, the `e2e` job)
  across Linux, macOS, and Windows rather than on every local `cargo test`.
- Before committing: `cargo test`, `cargo fmt --check`, and
  `cargo clippy --all-targets -- -D warnings` should all be clean. CI runs
  the same three checks, plus the end-to-end job, on every push and pull
  request.

## Commits

- Conventional Commits: `type: summary`, lowercase after the colon,
  imperative mood (`add`, not `added` or `adds`), for example
  `feat: add a sync command` or `perf: memoize ls_remote_sha within a run`.
  The types in use so far are `feat`, `fix`, `perf`, `docs`, and `chore`.
- The body explains why the change exists, the reasoning or trade-off behind it,
  not a restatement of the diff. Wrap prose around 76 columns.
- Each commit is a coherent, atomic step. Prefer several small commits that
  each build and pass tests over one large one that bundles unrelated
  changes.
