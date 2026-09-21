# Contributing to Broski

## Repository layout
- `crates/broski-cli`: CLI and command routing.
- `crates/broski-core`: DSL parser, graph, fingerprinting, executor.
- `crates/broski-store`: shared cache record types/traits.
- `crates/broski-cache`: local SQLite + CAS implementation.
- `examples/showcase`: React + Rust + Docker demonstration.

## Local setup
```bash
git clone https://github.com/himudigonda/Broski.git
cd Broski
cargo build --release -p broski-cli
```

## Required quality gate
Run before every PR:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
./target/release/broski --workspace . run ci
```

## CI policy
- PRs must be green in GitHub Actions before merge.
- Use feature branches, then merge straight to `main`.
- Cut release tags from green `main` only.

## Isolation guidance
- Generic tests should use host-agnostic isolation (`best_effort` or `off`) unless strict isolation is the feature under test.
- Linux strict isolation is validated in dedicated tests/jobs.

## Release references
- Docs Portal: <https://himudigonda.me/broski_docs/>
- Docs source: `website/`
- Security/architecture reports: `docs/reports/`

## Design expectations
- Determinism over convenience.
- Explicit contracts over implicit state.
- Atomic behavior over partial side effects.
- Actionable diagnostics over black-box behavior.
- Keep the DSL ergonomic and concise.
