# Changelog

All notable changes to this project are documented in this file.

## [0.6.1] - 2026-03-07

### Stabilization
- Validated `0.6.0` integration with full quality gates (`fmt`, `clippy`, workspace tests, docs build).
- Completed benchmark and smoke validation before release cut.

### Documentation and Positioning
- Repositioned README and quickstart docs around low-ceremony adoption: copy commands first, then add `@in` and `@out` for caching.
- Promoted `--explain` and rollback-safe output promotion as the primary differentiators.
- Added a `Progressive Adoption` section in the migration guide for Make/Just users.

### Hygiene
- Renamed `examples/basic/pleasefile` to `examples/basic/broskifile` to remove legacy naming drift.

### CI / Release Finalization
- Fixed showcase CI packaging contracts so `package_container` reliably resolves built artifacts in GitHub Actions.
- Fixed markdown lint spacing in quickstart docs (`MD032`) to keep `docs-ci` green.

## [0.5.2] - 2026-03-06

### Security
- Removed Windows `cmd.exe` fallback for default shell execution. Windows tasks now execute through PowerShell script files (`pwsh` preferred, `powershell` fallback) to reduce command-injection risk.
- Reworked interactive `@secret_env` redaction to stream process output (`spawn` + threaded readers) instead of buffering via `output()`, preventing interactive deadlocks and unbounded output buffering.
- Hardened strict Linux `bwrap` setup by replacing `--ro-bind / /` with explicit base directory mounts and `$HOME` tmpfs masking.
- Secret manifest entries now use keyed BLAKE3 with workspace-local salt from `.broski/config/salt` instead of unsalted fast hashes.

### Hardening
- Dynamic variable command capture now enforces a 1 MiB output cap with bounded draining to avoid memory blowups and pipe backpressure hangs.
- Runtime lock checks now include Linux process-start identity (`/proc/<pid>/stat` start ticks) to reduce false active-lock blocks from PID reuse.
- Cache restore now validates artifact relative paths and object hashes before writing into workspace paths.
- Added/updated regression tests for shell selection, strict isolation argument shape, redaction pattern floor, keyed secret fingerprints, parser output limits, and cache path/hash validation.

### Documentation
- Added deep-dive security hardening report at `docs/reports/v0.5.2-security-hardening.md` (includes Make/Just/Broski comparison and residual risk notes).
- Expanded docs portal coverage for migration-heavy teams:
  - dedicated `Anti-Patterns`, `Common Mistakes`, and `FAQ` pages under operations.
  - broader command/flag usage examples and corrected graph format docs (`text|dot`).
- Added attribution note to README and docs surfaces.

## [0.5.1] - 2026-03-05

### Changed
- Installer and release flow now use `broski-*` artifacts only.
- Docs route is canonical at `/broski_docs/`.
- Docs and README now use the new Broski banner/logo assets.
- CI and docs CI auto-run only on pushes to `main` (manual dispatch still available).
- Docs deploy workflow uses the correct Vercel execution path.

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
  - `@private` to hide tasks from `broski list`.
  - `@confirm` to require explicit user confirmation before execution.
- Built-in interpolation functions:
  - `{{ os() }}`, `{{ arch() }}`, `{{ env("KEY", "default") }}`.
- Release matrix publishes stable artifacts for Linux and macOS.
- Eclipse Portal documentation engine under `website/` (Docusaurus v3):
  - local search,
  - mermaid architecture visuals,
  - version dropdown with `v0.5` current and `v0.4` archived.

### Changed
- Workspace and crate version bumped to `0.5.0`.
- Root and all `examples/**/broskifile` files migrated to `version = "0.5"`.
- CLI parser + diagnostics now surface richer miette-backed source-span errors.
- Release/CI docs and runbook updated for stable `v0.5.0`.

### Compatibility
- TOML and DSL `0.3`/`0.4` continue to work in v0.5 with deprecation warnings.
- Legacy format removal target moved to v0.6.

## [0.4.0-rc.1] - 2026-03-05

### Added
- Implicit task execution via `broski <task>`.
- Native watch mode (`--watch`) for rerunning selected target graphs.
- DSL variable engine:
  - static declarations (`KEY = "value"`),
  - dynamic declarations (`KEY = $(...)`),
  - strict interpolation (`{{ KEY }}`).
- Task preflight requirements via `@requires`.
- Secret redaction for interactive terminal output and persisted logs.
- Task descriptions parsed from preceding comments and shown in `broski list`.

### Changed
- DSL now supports `version = "0.4"` as the primary format.
- Root and example `broskifile`s migrated to `version = "0.4"`.
- DSL `version = "0.3"` now emits a deprecation warning in v0.4.
- Fingerprints now include resolved variable values used by tasks.
- Installer default channel now resolves latest published release (`BROSKI_CHANNEL=latest`).
- Stable-only installs are available via `BROSKI_CHANNEL=stable`.

### Compatibility
- TOML `broskifile` support remains available with deprecation warning (planned removal target: v0.5).

## [0.3.0-beta.1] - 2026-03-05

### Added
- Hybrid task execution model:
  - graph mode (cached/staged/deterministic).
  - interactive mode (live workspace + TTY + uncached).
- DSL-first `broskifile` support (`version = "0.3"`) with:
  - task headers (`task: deps...`),
  - annotations (`@in`, `@out`, `@env`, `@secret_env`, `@dir`, `@mode`, `@isolation`),
  - aliases (`alias short = target`),
  - global env loading (`@load .env`).
- CLI passthrough arguments:
  - `broski run <task> -- <args...>`.
- Mode-aware explain diagnostics:
  - interactive bypass reason surfaced in `--explain`.
- Alias-aware execution:
  - aliases resolve in `run`, `graph`, and `list`.

### Changed
- Parser autodetect now defaults to DSL and falls back to TOML when TOML sections are detected.
- TOML parser path now emits a deprecation warning (removal target: v0.5).
- Root and example `broskifile`s migrated to DSL v0.3.
- CLI integration fixtures migrated to DSL and expanded for:
  - interactive explain behavior,
  - passthrough fingerprint delta behavior,
  - alias invocation in `run` and `graph`.

### Compatibility
- TOML `broskifile` support is retained in v0.3 for safe migration.

## [0.2.0] - 2026-03-04

### Added
- Cache telemetry with `broski run <task> --explain`.
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
