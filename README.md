# Broski

![Broski Banner](website/static/img/branding/broski_banner.png)

[![Version](https://img.shields.io/badge/version-v0.5.2-blue)](https://github.com/himudigonda/Broski/releases/tag/v0.5.2)
[![CI](https://img.shields.io/github/actions/workflow/status/himudigonda/Broski/ci.yml?branch=main&label=build)](https://github.com/himudigonda/Broski/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-green)](#license)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange)](https://www.rust-lang.org/)

Deterministic task runner and build orchestrator for teams replacing Make/Just in local + CI workflows.

## Install in 10 Seconds

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | bash
broski --version
```

Pinned install:

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | BROSKI_VERSION=v0.5.2 bash
```

## Why Broski

| Capability | Make | Just | Broski |
| --- | --- | --- | --- |
| Content-based invalidation | No | No | Yes (BLAKE3) |
| Cache miss explainability | No | No | Yes (`--explain`) |
| ACID-safe output promotion | No | No | Yes |
| Interactive + graph modes | Basic | Basic | First-class |
| Dependency DAG orchestration | Partial | Limited | Full target graph |

## Quickstart

```bash
broski --workspace . list
broski --workspace . ci
broski --workspace . run ci --explain
broski --workspace . run test --watch
```

## Docs Portal

- Public docs: [https://himudigonda.me/broski_docs/](https://himudigonda.me/broski_docs/)
- Standalone docs origin: [https://broski-docs.vercel.app/broski_docs/](https://broski-docs.vercel.app/broski_docs/)

## Highlights in v0.5

- first-class task parameters (`task [arg] [arg="default"]:`)
- modular imports (`@import`)
- decorators (`@private`, `@confirm`)
- built-in interpolation (`os()`, `arch()`, `env()`)
- shebang task bodies

## Repo Layout

- `crates/` — core engine, CLI, cache, store
- `broskifile` — dogfooding orchestration
- `website/` — docs portal (Docusaurus)
- `examples/` — runnable end-to-end samples

## Developer Workflow

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
./target/debug/broski --workspace . run ci --explain
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
broski --help
broski doctor --no-repair
```

Then check the portal troubleshooting and architecture sections.

## License

MIT. See `LICENSE`.

This codebase was built with help from Codex-5.3-Extra-High. Gemini 3.1 Pro helped with the docs.
