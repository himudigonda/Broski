# Contributing to Please

Thanks for contributing.

## Repository layout
- `crates/please-cli`: CLI argument parsing and user-facing output.
- `crates/please-core`: parser, graph planner, fingerprinting, executor.
- `crates/please-store`: storage trait + shared execution/cache record types.
- `crates/please-cache`: local SQLite + CAS artifact store.
- `examples/showcase`: React + Rust + Docker proof-of-value application.

## Local setup
```bash
git clone https://github.com/himudigonda/Please.git
cd Please
cargo build --release -p please-cli
```

## Required quality gate
Run before every PR:
```bash
./target/release/please --workspace . run ci
```
If you prefer using a debug build during active development:
```bash
cargo run -p please-cli -- --workspace . run ci
```

## CI no-break policy
- PRs must be green in GitHub Actions before merge.
- Feature work should land via PR (no direct pushes to `main`).
- Release tags should be cut from already-green `main` commits only.

## Isolation guidance for tests
- Generic integration tests must set explicit `@isolation best_effort` (or TOML `isolation = "best_effort"` for legacy fixtures) unless strict isolation is the feature under test.
- Strict-isolation behavior should be tested in dedicated Linux-focused tests.

## Expected test coverage by change type
- Parser/config changes: parser + validator tests.
- Cache/fingerprint changes: unit tests + integration miss/hit assertions.
- Executor behavior changes: ACID regression tests and explain-path tests.
- Showcase changes: local build/package validation and endpoint smoke checks.

## Release process
See:
- [docs/release-runbook.md](docs/release-runbook.md)
- [docs/release-v0.4.md](docs/release-v0.4.md)

## Design principles
- Determinism over convenience.
- Explicit contracts over implicit state.
- Atomic behavior over partial side effects.
- Actionable diagnostics over black-box execution.
- Keep the DSL ergonomic; prefer simple task bodies with annotations only where needed.
