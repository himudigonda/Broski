# Contributing to Please

Thanks for contributing.

## Repository layout
- `crates/please-cli`: CLI commands, argument parsing, user-facing diagnostics.
- `crates/please-core`: config parsing, graph planner, fingerprinting, executor.
- `crates/please-store`: shared artifact-store trait and storage record types.
- `crates/please-cache`: local SQLite + CAS implementation.
- `docs/`: architecture, migration, release process.

## Local setup
```bash
git clone https://github.com/himudigonda/Please.git
cd Please
just setup
```

## Development workflow
1. Make your change.
2. Run quality gate:
```bash
just ci
```
3. Optional equivalent via Please:
```bash
please --workspace . run ci
```
4. Open a PR against `main`.

## Quality gates
All PRs should keep these green:
- formatting: `cargo fmt --all --check`
- lints: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- tests: `cargo nextest run --workspace --all-features` (fallback `cargo test`)
- coverage: `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines ${PLEASE_COVERAGE_MIN:-45}`

## Tests expected with changes
- Bug fixes should include regression tests.
- Parser/config changes should add parser or validator tests.
- Executor/cache behavior changes should add integration coverage in `crates/please-cli/tests/e2e.rs` when possible.

## Release and versioning
- Release process is documented in [docs/release-runbook.md](/Users/himudigonda/Documents/Projects/Please/docs/release-runbook.md).
- Tag format: `v<semver>` (example `v0.1.0-alpha.1`).

## Design goals
- Deterministic invalidation via content hashes.
- Explicit task contracts.
- Transactional output promotion.
- Predictable cross-platform behavior (Linux + macOS).

If you are unsure whether a change fits these goals, open an issue before implementing a large refactor.
