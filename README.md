# Please

`Please` is a deterministic task runner and build orchestrator designed to replace `make` and `just` for local, CI, and mid-size polyglot projects.

## Status
- Stable: `v0.5.0`
- Compatibility retained in v0.5: DSL `0.3`/`0.4` and TOML (deprecated, removal target `v0.6`)

## Why Please
- Content-based invalidation (BLAKE3), not mtime heuristics.
- DAG scheduling for parallel graph tasks.
- Interactive mode for dev servers and ad-hoc commands.
- Transactional output promotion (ACID-safe output handling).
- Cache telemetry with `--explain`.

## Install
Published artifacts:
- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Install latest stable (default):
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

Install a pinned version:
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.5.0 bash
```

## CLI quickstart
```bash
please --workspace . list
please --workspace . ci
please --workspace . run ci --explain
please --workspace . run test --watch
please --workspace . graph ci --format text
```

## v0.5 DSL quickstart
```text
version = "0.5"
alias b = build

# Reusable task with params
build [target] [mode="release"]:
    @in src/**/* Cargo.toml
    @out dist/{{ target }}
    @requires cargo
    @isolation off
    cargo build --bin {{ target }} --{{ mode }}
    cp target/{{ mode }}/{{ target }} dist/{{ target }}

# Hidden helper
internal_clean:
    @private
    rm -rf dist

# Guarded deploy
publish:
    @confirm "Publish release artifacts? [y/N]"
    ./scripts/publish.sh

# Interactive task
web:
    @mode interactive
    npm run dev
```

## v0.5 highlights
- First-class task parameters (`task [arg] [arg="default"]:`).
- `@import` for modular multi-file task definitions.
- Decorators: `@private`, `@confirm`.
- Built-ins in interpolation: `{{ os() }}`, `{{ arch() }}`, `{{ env("KEY", "default") }}`.
- Shebang task bodies (`#!`) for embedded polyglot scripts.
- Subcommand-first CLI precedence with implicit task run.
- Windows shell support (`pwsh` first, `cmd` fallback).

## Compatibility
- TOML `pleasefile` (deprecated warning).
- DSL `version = "0.3"` and `version = "0.4"` (deprecated warning).
- Removal target for both legacy paths: `v0.6`.

## Examples
`examples/` includes:
- `basic`
- `minimal`
- `polyglot`
- `python-cli`
- `go-http`
- `node-web`
- `showcase`

Smoke examples:
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
- [docs/dsl-v0.5-reference.md](docs/dsl-v0.5-reference.md)
- [docs/migration.md](docs/migration.md)
- [docs/security.md](docs/security.md)
- [docs/showcase.md](docs/showcase.md)
- [docs/release-v0.5.md](docs/release-v0.5.md)
- [docs/release-runbook.md](docs/release-runbook.md)
