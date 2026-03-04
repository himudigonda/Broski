# Please v0.1 Release Notes (Draft)

## Included
- `please run`, `please list`, `please graph`, `please doctor`, `please cache prune`.
- TOML `pleasefile` parser + semantic validation.
- Deterministic DAG execution with parallel per-layer scheduling.
- Local CAS and SQLite metadata cache.
- Staged execution with transactional output promotion.
- Linux strict isolation via `bwrap`; macOS best-effort isolation.

## Known limitations
- No remote cache backend in v0.1.
- Stage snapshot currently copies workspace files (correctness-first, not optimized).
- Coverage gate default in `just ci` is bootstrap-oriented (`PLEASE_COVERAGE_MIN`, default 45).
- Winnow parser path is opt-in (`PLEASE_PARSER_MODE=winnow`); TOML remains default.
