# Please v0.2.0 Release Notes

## Release type
- Tag: `v0.2.0`
- Channel: stable (pre-1.0)
- Repository: `himudigonda/Please`

## Highlights
- Added cache telemetry with `please run <task> --explain`.
- Persisted fingerprint manifest metadata in cache execution records.
- Added latest-execution lookup and miss delta diagnostics.
- Split and expanded CLI integration suite with fixture-backed tests.
- Added Linux CI hardening for bubblewrap and host-agnostic generic e2e isolation.
- Added full showcase project (`examples/showcase`) with React + Axum + Docker orchestration via Please.

## User-facing changes
- New run flag: `--explain`.
- Improved miss/bypass diagnostics:
  - `cache miss: input changed: ...`
  - `cache bypass: --force supplied`

## Validation evidence
- `cargo run -p please-cli -- --workspace . run ci` green locally.
- `please --workspace . run ci` green locally.
- Showcase tasks validated:
  - `build_ui`
  - `build_api`
  - `package_container`

## Known limitations
- No remote/shared cache backend yet.
- No automated Makefile/justfile importer (manual migration only).
- Strict sandboxing remains Linux-only.
