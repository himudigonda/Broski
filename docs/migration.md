# Migration Guide: `make` / `just` to `please` (v0.4 DSL)

`Please` replaces workflow orchestration semantics, not Make/Just syntax parsing.

## Important
There is intentionally **no automatic importer** in v0.4.
Manual migration keeps task contracts explicit where you want deterministic caching.

## Concept mapping
| Concept | make | just | please v0.4 DSL |
| --- | --- | --- | --- |
| Task definition | target | recipe | `task_name:` |
| Dependencies | prerequisites | dependencies | `task: dep_a dep_b` |
| Inputs | implicit/mtime | implicit | `@in ...` |
| Outputs | implicit | implicit | `@out ...` |
| Runtime env | shell env | vars/dotenv | `@env`, `@secret_env`, `@load .env` |
| Variable reuse | make vars | just vars | `KEY = "value"`, `{{ KEY }}` |
| Tool prechecks | manual | manual | `@requires ...` |
| Rebuild logic | timestamps | rerun by default | content fingerprint + cache |
| Dev server mode | phony target | normal recipe | `@mode interactive` |

## Translation checklist
1. Create one DSL task per legacy target/recipe.
2. Keep shell commands as normal indented lines (no quoted TOML strings).
3. Add deps in the task header.
4. Add all cache-relevant source/config patterns to `@in`.
5. Add concrete artifacts/stamps to `@out` for graph tasks.
6. Use `@mode interactive` for long-running/local dev tasks.
7. Validate with:
   - `please --workspace . doctor`
   - `please --workspace . run <task> --explain`

## Example: make -> please
```make
build: src/main.rs Cargo.toml
	cargo build --release
```

```text
version = "0.4"

build:
    @in src/main.rs Cargo.toml
    @out target/release/app
    @isolation off
    cargo build --release
```

## Example: just -> please
```just
lint:
  cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```text
version = "0.4"

lint:
    @in Cargo.toml crates/**/*.rs
    @out .please/stamps/lint.ok
    @isolation off
    mkdir -p .please/stamps
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    printf 'ok\n' > .please/stamps/lint.ok
```

## TOML compatibility
Legacy TOML files still work in v0.4 with a deprecation warning. Migrate before v0.5.

## Dynamic variable caution
Dynamic variables (`KEY = $(...)`) are powerful, but nondeterministic commands like `$(date)` or
`$(uuidgen)` will force frequent cache misses. Prefer deterministic commands tied to repository
state (for example: `$(git rev-parse HEAD)`).

## Debugging misses
Use explain mode to identify exactly what changed:
```bash
please --workspace . run lint --explain
```
