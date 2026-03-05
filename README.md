# Please

`Please` is a deterministic task runner for polyglot projects.

## Status
- Stable channel: `v0.2.0`
- Current prerelease: `v0.4.0-rc.1` (Intelligence + Ergonomics)

## Why Please
- Content-hash invalidation (BLAKE3), not mtime heuristics.
- DAG-aware parallel scheduling.
- Transactional output promotion.
- Local CAS + SQLite execution metadata.
- Explainable cache misses via `--explain`.
- Interactive mode for dev servers and ad-hoc commands.

## Install
Supported binaries:
- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

Install latest stable (default):
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

Install a pinned beta:
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.4.0-rc.1 bash
```

## CLI quickstart
```bash
please --workspace . list
please --workspace . run ci
please --workspace . ci
please --workspace . run ci --explain
please --workspace . graph ci --format text
please --workspace . run test -- --watch
```

## v0.4 DSL quickstart
Create a `pleasefile`:
```text
version = "0.4"
alias b = build
RUST_TARGET = "target/release/app"

# Build release artifact
build:
    @in src/**/* Cargo.toml
    @out {{ RUST_TARGET }}
    @requires cargo
    @isolation off
    cargo build --release && cp target/release/app {{ RUST_TARGET }}

dev:
    @mode interactive
    @isolation off
    npm run dev
```

Run:
```bash
please --workspace . b --explain
please --workspace . run b --explain
please --workspace . dev
please --workspace . run test --watch
```

Watch mode note:
- `--watch` reruns target graphs on input changes and ignores `.git`, `.please`, and declared outputs.
- For interactive tasks that already have internal watchers (for example Vite), Please prints a conflict warning.

## TOML compatibility
TOML `pleasefile`s from v0.1/v0.2 still run in v0.4 with a deprecation warning.
Migration target for TOML removal is v0.5.

## Showcase (React + Rust + Docker)
```bash
cd examples/showcase
../../target/debug/please --workspace . run package_container --explain
../../target/debug/please --workspace . run prove_cache
```

See:
- [docs/showcase.md](docs/showcase.md)
- [examples/showcase/README.md](examples/showcase/README.md)

## Examples
Language/framework examples live under `examples/`:
- `basic`
- `minimal`
- `polyglot`
- `python-cli`
- `go-http`
- `node-web`
- `showcase`

Smoke all non-Docker examples:
```bash
please --workspace . run examples_smoke --explain
```

## Developer quickstart
```bash
cargo build --release -p please-cli
./target/release/please --workspace . run ci
```

## Docs
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CHANGELOG.md](CHANGELOG.md)
- [docs/architecture.md](docs/architecture.md)
- [docs/cache-telemetry.md](docs/cache-telemetry.md)
- [docs/migration.md](docs/migration.md)
- [docs/security.md](docs/security.md)
- [docs/release-v0.4.md](docs/release-v0.4.md)
