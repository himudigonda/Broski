# Broski

![Broski Banner](website/static/img/branding/broski_banner.png)

[![Version](https://img.shields.io/badge/version-v0.6.1-blue)](https://github.com/himudigonda/Broski/releases/tag/v0.6.1)
[![CI](https://img.shields.io/github/actions/workflow/status/himudigonda/Broski/ci.yml?branch=main&label=build)](https://github.com/himudigonda/Broski/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-green)](#license)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange)](https://www.rust-lang.org/)

A drop-in task runner with Justfile simplicity, out-of-the-box caching, and `--explain` for broken builds.

Early release note: Broski is production-usable for local/small-team workflows, but still in active alpha/beta iteration.

## 30-Second Example

Start with plain commands:

```bash title="broskifile"
version = "0.5"

test:
    python3 -m unittest
```

Then add two lines for reusable graph execution:

```bash title="broskifile"
version = "0.5"

test:
    @in src/**/*.py tests/**/*.py
    @out .broski/stamps/test.ok
    mkdir -p .broski/stamps
    python3 -m unittest
    printf 'ok\n' > .broski/stamps/test.ok
```

Run and inspect:

```bash
broski run test
broski run test --explain
```

Typical explain output:
- `cache hit` when inputs are unchanged
- `cache miss: input changed: tests/test_api.py` when content changed

## Why Broski

- `--explain` tells you exactly why a task reran.
- Transactional output promotion avoids poisoned workspace state on failures.
- Interactive and graph execution are both first-class (`@mode interactive` for long-running dev tasks, graph mode for cacheable artifact tasks).

## Who This Is For Right Now

- Solo developers and small teams with messy shell scripts.
- Make/Just users who want cache explainability and safer output handling.
- Repos where rerun debugging costs real time.

## Not Trying To Be Everything Yet

- No remote/shared cache in this release line.
- Not positioned yet as a full enterprise orchestration platform.
- DSL and docs are still being trimmed for lower ceremony.

## Install in 10 Seconds

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | bash
broski --version
```

Pinned install:

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | BROSKI_VERSION=v0.6.1 bash
```

## Docs Portal

- Public docs: [https://himudigonda.me/broski_docs/](https://himudigonda.me/broski_docs/)
- Standalone docs origin: [https://broski-docs.vercel.app/broski_docs/](https://broski-docs.vercel.app/broski_docs/)

## Repo Layout

- `crates/` - core engine, CLI, cache, store
- `broskifile` - dogfooding orchestration
- `website/` - docs portal (Docusaurus)
- `examples/` - runnable end-to-end samples

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
