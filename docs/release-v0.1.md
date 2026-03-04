# Please v0.1.0-alpha.1 Release Notes (Draft)

## Release type
- Tag: `v0.1.0-alpha.1`
- Channel: prerelease
- Repository: `himudigonda/Please`

## Included
- `please run`, `please list`, `please graph`, `please doctor`, `please cache prune`.
- TOML `pleasefile` parser + semantic validation.
- Dual parser mode support (`PLEASE_PARSER_MODE=toml|winnow`, TOML default).
- Deterministic DAG execution with parallel per-layer scheduling.
- Local CAS + SQLite metadata cache.
- Staged execution with transactional output promotion.
- Linux strict isolation via `bwrap`; macOS best-effort isolation.

## Published artifacts
- `please-v0.1.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz`
- `please-v0.1.0-alpha.1-aarch64-apple-darwin.tar.gz`
- `SHA256SUMS.txt`

## Local build baseline
- Release profile: `lto=true`, `codegen-units=1`, `strip=true`, `panic=abort`.
- Local macOS arm64 binary size (`target/release/please`): `2.9M`.
- Linux x64 artifact size: pending release workflow run.
- Local startup smoke check (`please --help`): `real 0.05s`, peak memory footprint `~1.1 MB`.

## Known limitations
- Coverage is currently below long-term production target; more hardening tests are planned.
- No remote cache backend in v0.1.x.
- Telemetry/tracing for cache-miss diagnostics is not implemented yet.
- Strict sandboxing is Linux-only.
- Installer supports only Linux x86_64 and macOS arm64 in this alpha.
