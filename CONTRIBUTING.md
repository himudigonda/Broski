# Contributing to Please

## Repository layout
- `crates/please-cli`: CLI and command routing.
- `crates/please-core`: DSL parser, graph, fingerprinting, executor.
- `crates/please-store`: shared cache record types/traits.
- `crates/please-cache`: local SQLite + CAS implementation.
- `examples/showcase`: React + Rust + Docker demonstration.

## Local setup
```bash
git clone https://github.com/himudigonda/Please.git
cd Please
cargo build --release -p please-cli
```

## Required quality gate
Run before every PR:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
./target/release/please --workspace . run ci
```

## CI policy
- PRs must be green in GitHub Actions before merge.
- Use feature branches, then merge to `develop`.
- Merge `develop` to `main` only when release-ready.
- Cut release tags from green `main` only.

## Isolation guidance
- Generic tests should use host-agnostic isolation (`best_effort` or `off`) unless strict isolation is the feature under test.
- Linux strict isolation is validated in dedicated tests/jobs.

## Release references
- [docs/release-runbook.md](docs/release-runbook.md)
- [docs/release-v0.5.md](docs/release-v0.5.md)

## Design expectations
- Determinism over convenience.
- Explicit contracts over implicit state.
- Atomic behavior over partial side effects.
- Actionable diagnostics over black-box behavior.
- Keep the DSL ergonomic and concise.
