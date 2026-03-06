# Please

[![Version](https://img.shields.io/badge/version-v0.5.0-blue)](https://github.com/himudigonda/Please/releases/tag/v0.5.0)
[![CI](https://img.shields.io/github/actions/workflow/status/himudigonda/Please/ci.yml?branch=main&label=build)](https://github.com/himudigonda/Please/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-green)](#license)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange)](https://www.rust-lang.org/)

Deterministic task runner and build orchestrator for teams replacing Make/Just in local + CI workflows.

## Install in 10 Seconds

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
please --version
```

Pinned install:

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.5.0 bash
```

## Why Please

| Capability | Make | Just | Please |
| --- | --- | --- | --- |
| Content-based invalidation | No | No | Yes (BLAKE3) |
| Cache miss explainability | No | No | Yes (`--explain`) |
| ACID-safe output promotion | No | No | Yes |
| Interactive + graph modes | Basic | Basic | First-class |
| Dependency DAG orchestration | Partial | Limited | Full target graph |

## Quickstart

```bash
please --workspace . list
please --workspace . ci
please --workspace . run ci --explain
please --workspace . run test --watch
```

## Docs Portal

- Public docs: [https://himudigonda.me/please_docs/](https://himudigonda.me/please_docs/)
- Standalone docs origin: [https://please-docs.vercel.app/please_docs/](https://please-docs.vercel.app/please_docs/)

## Highlights in v0.5

- first-class task parameters (`task [arg] [arg="default"]:`)
- modular imports (`@import`)
- decorators (`@private`, `@confirm`)
- built-in interpolation (`os()`, `arch()`, `env()`)
- shebang task bodies
- compatibility retained: DSL `0.3`/`0.4` and TOML (deprecated; removal target `v0.6`)

## Repo Layout

- `crates/` — core engine, CLI, cache, store
- `pleasefile` — dogfooding orchestration
- `website/` — docs portal (Docusaurus)
- `docs/legacy/` — archived markdown docs
- `examples/` — runnable end-to-end samples

## Developer Workflow

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
./target/debug/please --workspace . run ci --explain
```

Docs workflow:

```bash
cd website
npm ci
npm run lint:all
```

## Support

If a command fails, run:

```bash
please --help
please doctor --no-repair
```

Then check the portal troubleshooting and architecture sections.

## License

MIT. See `LICENSE`.
