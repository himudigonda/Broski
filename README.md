# Please

`Please` is a deterministic task runner for polyglot projects.

## Alpha status
`Please` is currently in **alpha**. The first public prerelease is `v0.1.0-alpha.1`.
Use it for dogfooding and feedback. Production-hardening work is ongoing.

## What you get in v0.1
- TOML `pleasefile` parsing with semantic validation.
- DAG scheduling with deterministic topological layers.
- Content-hash fingerprints (BLAKE3) for task invalidation.
- Local CAS + SQLite cache metadata.
- Staged execution with transactional output promotion.
- Isolation policy support:
  - Linux: strict isolation via `bwrap`.
  - macOS: best-effort isolation (strict mode unsupported).

## Install
Supported release binaries:
- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

### Clone-and-install flow
```bash
git clone https://github.com/himudigonda/Please.git
cd Please
./install.sh
please --version
```

### Curl install flow
```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

Install a specific version:
```bash
PLEASE_VERSION=v0.1.0-alpha.1 ./install.sh
```

Default install dir is `~/.local/bin` (`INSTALL_DIR` can override).

## Usage
```bash
please --workspace . list
please --workspace . run ci
please --workspace . graph ci --format text
```

## Drop-in migration path
`Please` is a workflow replacement for `make`/`just` rather than a syntax-compatible parser.
Use the migration guide to translate existing recipes into explicit `inputs`/`outputs` task contracts:
- [Migration guide](/Users/himudigonda/Documents/Projects/Please/docs/migration.md)
- [Polyglot example](/Users/himudigonda/Documents/Projects/Please/examples/polyglot/pleasefile)
- [Minimal example](/Users/himudigonda/Documents/Projects/Please/examples/minimal/pleasefile)

## Contributor quick start
```bash
just setup
just ci
```

You can also run the same gate through Please:
```bash
please --workspace . run ci
```

Contribution process and architecture notes:
- [Contributing guide](/Users/himudigonda/Documents/Projects/Please/CONTRIBUTING.md)
- [Architecture deep dive](/Users/himudigonda/Documents/Projects/Please/docs/architecture.md)
- [Release runbook](/Users/himudigonda/Documents/Projects/Please/docs/release-runbook.md)

## Parser mode
- Default parser: TOML (`PLEASE_PARSER_MODE=toml`).
- Experimental parser path: `PLEASE_PARSER_MODE=winnow`.
