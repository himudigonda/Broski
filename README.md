# Please

`Please` is a deterministic task runner for polyglot projects.

## v0.1 capabilities
- TOML `pleasefile` parsing with semantic validation.
- DAG scheduling with deterministic topological layers.
- Content-hash fingerprints (BLAKE3) for task invalidation.
- Local CAS + SQLite cache metadata.
- Staged execution with transactional output promotion.
- Linux strict isolation via `bwrap` and macOS best-effort isolation.

## Quick start
```bash
just setup
just ci
just run -- list
```

## Example
- Minimal runnable demo: [`examples/basic/pleasefile`](/Users/himudigonda/Documents/Projects/Please/examples/basic/pleasefile)
- Polyglot template: [`examples/polyglot/pleasefile`](/Users/himudigonda/Documents/Projects/Please/examples/polyglot/pleasefile)

## Coverage Gate
- `just ci` enforces coverage through `cargo llvm-cov`.
- Override threshold with `PLEASE_COVERAGE_MIN` (default `45` during bootstrap).

## Parser Mode
- Default parser: TOML (`PLEASE_PARSER_MODE=toml`).
- Experimental parser path: `PLEASE_PARSER_MODE=winnow`.
