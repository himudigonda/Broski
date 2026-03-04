# Changelog

All notable changes to this project are documented in this file.

## [0.2.0-beta.1] - 2026-03-04

### Added
- Cache telemetry with `please run <task> --explain`.
- Fingerprint manifest generation and manifest-aware cache miss diagnostics.
- Execution record manifest persistence and latest-execution lookup in local cache.
- Fixture-backed modular CLI integration test suite:
  - `e2e_cache`, `e2e_acid`, `e2e_doctor`, `e2e_explain`, `e2e_graph`.
- End-to-end showcase app under `examples/showcase`:
  - Vite + React dashboard frontend.
  - Axum backend with `/api/health` and `/api/metrics`.
  - Docker packaging task and cache proof script.
- CI showcase validation job (build UI/API, smoke API health, package container).

### Changed
- Generic e2e fixtures now set `isolation = "best_effort"` explicitly.
- Linux CI now installs and verifies `bwrap`.
- Workspace version bumped to `0.2.0-beta.1`.
- Release and migration docs updated for beta and manual-first migration guidance.

### Fixed
- Installer/runtime edge cases validated through end-to-end release flow.
- Deterministic cache explanation output for changed/added/removed manifest keys.

## [0.1.0-alpha.1] - 2026-03-04

### Added
- Initial deterministic task runner alpha:
  - `run`, `list`, `graph`, `doctor`, `cache prune`.
  - DAG-aware execution, local CAS + SQLite cache.
  - Transactional output promotion.
  - Linux strict isolation support and macOS best-effort mode.

