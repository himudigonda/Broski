# Changelog

All notable changes to this project are documented in this file.

## [0.5.0] - 2026-03-05

### Added
- First-class DSL task parameters:
  - `task [arg] [arg="default"]`.
- `@import` directive with circular import detection and depth guardrails.
- Name-collision protection across imported task/alias/variable declarations.
- Shebang task execution with temporary script lifecycle cleanup.
- Cross-platform shell command resolution:
  - POSIX `/bin/sh` on Unix.
  - `pwsh` first, `cmd` fallback on Windows.
- Decorators:
  - `@private` to hide tasks from `please list`.
  - `@confirm` to require explicit user confirmation before execution.
- Built-in interpolation functions:
  - `{{ os() }}`, `{{ arch() }}`, `{{ env("KEY", "default") }}`.
- Release matrix now publishes Windows artifact (`x86_64-pc-windows-msvc`).

### Changed
- Workspace and crate version bumped to `0.5.0`.
- Root and all `examples/**/pleasefile` files migrated to `version = "0.5"`.
- CLI parser + diagnostics now surface richer miette-backed source-span errors.
- Release/CI docs and runbook updated for stable `v0.5.0`.

### Compatibility
- TOML and DSL `0.3`/`0.4` continue to work in v0.5 with deprecation warnings.
- Legacy format removal target moved to v0.6.

## [0.4.0-rc.1] - 2026-03-05

### Added
- Implicit task execution via `please <task>`.
- Native watch mode (`--watch`) for rerunning selected target graphs.
- DSL variable engine:
  - static declarations (`KEY = "value"`),
  - dynamic declarations (`KEY = $(...)`),
  - strict interpolation (`{{ KEY }}`).
- Task preflight requirements via `@requires`.
- Secret redaction for interactive terminal output and persisted logs.
- Task descriptions parsed from preceding comments and shown in `please list`.

### Changed
- DSL now supports `version = "0.4"` as the primary format.
- Root and example `pleasefile`s migrated to `version = "0.4"`.
- DSL `version = "0.3"` now emits a deprecation warning in v0.4.
- Fingerprints now include resolved variable values used by tasks.

### Compatibility
- TOML `pleasefile` support remains available with deprecation warning (planned removal target: v0.5).

## [0.3.0-beta.1] - 2026-03-05

### Added
- Hybrid task execution model:
  - graph mode (cached/staged/deterministic).
  - interactive mode (live workspace + TTY + uncached).
- DSL-first `pleasefile` support (`version = "0.3"`) with:
  - task headers (`task: deps...`),
  - annotations (`@in`, `@out`, `@env`, `@secret_env`, `@dir`, `@mode`, `@isolation`),
  - aliases (`alias short = target`),
  - global env loading (`@load .env`).
- CLI passthrough arguments:
  - `please run <task> -- <args...>`.
- Mode-aware explain diagnostics:
  - interactive bypass reason surfaced in `--explain`.
- Alias-aware execution:
  - aliases resolve in `run`, `graph`, and `list`.

### Changed
- Parser autodetect now defaults to DSL and falls back to TOML when TOML sections are detected.
- TOML parser path now emits a deprecation warning (removal target: v0.5).
- Root and example `pleasefile`s migrated to DSL v0.3.
- CLI integration fixtures migrated to DSL and expanded for:
  - interactive explain behavior,
  - passthrough fingerprint delta behavior,
  - alias invocation in `run` and `graph`.

### Compatibility
- TOML `pleasefile` support is retained in v0.3 for safe migration.

## [0.2.0] - 2026-03-04

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
- Workspace version bumped to `0.2.0`.
- Release and migration docs updated for v0.2.0 and manual-first migration guidance.

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
