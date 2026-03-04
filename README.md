# Please

`Please` is a deterministic task runner for polyglot projects with explicit task contracts (`inputs`, `outputs`, `deps`, `env`, `run`).

## Status
Current prerelease target: **`v0.2.0-beta.1`**.

## Why Please
- Content-hash invalidation (BLAKE3), not mtime heuristics.
- DAG-aware parallel scheduling.
- Transactional output promotion.
- Local CAS + SQLite execution metadata.
- Explainable cache misses via `--explain`.

## Install
Supported binaries:
- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

Clone-and-install:
```bash
git clone https://github.com/himudigonda/Please.git
cd Please
./install.sh
please --version
```

Curl install:
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

Install specific version:
```bash
PLEASE_VERSION=v0.2.0-beta.1 ./install.sh
```

## CLI quickstart
```bash
please --workspace . list
please --workspace . run ci
please --workspace . run ci --explain
please --workspace . graph ci --format text
```

## Cache explain mode
Use `--explain` when you expect a cache hit but see execution:
```bash
please --workspace . run build_api --explain
```
Example reasons:
- `cache miss: input changed: src/main.rs`
- `cache miss: env changed: MODE`
- `cache bypass: --no-cache supplied`

## Showcase (React + Rust + Docker)
`examples/showcase` proves end-to-end orchestration.

Build and package:
```bash
cd examples/showcase
../../target/debug/please --workspace . run package_container --explain
```

Run cache proof script:
```bash
../../target/debug/please --workspace . run prove_cache
```

See:
- [docs/showcase.md](docs/showcase.md)
- [examples/showcase/README.md](examples/showcase/README.md)

## Migration note
Please is a workflow replacement for `make`/`just`, not a syntax parser for their files.
Manual-first migration is intentional to preserve deterministic contracts.
See [docs/migration.md](docs/migration.md).

## Developer quickstart
```bash
just setup
just ci
please --workspace . run ci
```

## Core docs
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CHANGELOG.md](CHANGELOG.md)
- [docs/architecture.md](docs/architecture.md)
- [docs/cache-telemetry.md](docs/cache-telemetry.md)
- [docs/release-v0.2.md](docs/release-v0.2.md)
