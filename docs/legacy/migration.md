# Migration Guide: `make` / `just` to `please` (v0.5 DSL)

`Please` replaces workflow orchestration semantics, not Make/Just syntax parsing.

## Important
There is intentionally **no automatic importer** in v0.5.
Manual migration keeps task contracts explicit where deterministic caching matters.

## Concept mapping
| Concept | make | just | please v0.5 DSL |
| --- | --- | --- | --- |
| Task definition | target | recipe | `task_name:` |
| Dependencies | prerequisites | dependencies | `task: dep_a dep_b` |
| Parameters | vars / env | recipe params | `task [arg] [arg="default"]:` |
| Inputs | implicit/mtime | implicit | `@in ...` |
| Outputs | implicit | implicit | `@out ...` |
| Runtime env | shell env | vars/dotenv | `@env`, `@secret_env`, `@load .env` |
| Reusable values | make vars | just vars | `KEY = "value"`, built-ins, `{{ KEY }}` |
| Tool prechecks | manual | manual | `@requires ...` |
| Rebuild logic | timestamps | rerun by default | content fingerprint + cache |
| Dev server mode | phony target | normal recipe | `@mode interactive` |
| File modularity | include | import/mod | `@import path/to/pleasefile` |

## Translation checklist
1. Create one DSL task per legacy target/recipe.
2. Keep shell commands as normal indented lines.
3. Add deps in task headers.
4. Add all cache-relevant source/config patterns to `@in`.
5. Add artifacts/stamps to `@out` for graph tasks.
6. Use `@mode interactive` for long-running dev tasks.
7. Add `@requires` where prerequisites are often missing.
8. Validate with:
   - `please --workspace . doctor`
   - `please --workspace . run <task> --explain`

## Example: make -> please
```make
build: src/main.rs Cargo.toml
	cargo build --release
```

```text
version = "0.5"

build:
    @in src/main.rs Cargo.toml
    @out target/release/app
    cargo build --release
```

## Example: just params -> please params
```just
build target mode="release":
    cargo build --bin {{target}} --{{mode}}
```

```text
version = "0.5"

build [target] [mode="release"]:
    cargo build --bin {{ target }} --{{ mode }}
```

## Compatibility policy in v0.5
- DSL `version = "0.3"` and `version = "0.4"` still work with deprecation warnings.
- TOML `pleasefile` still works with deprecation warning.
- Removal target for both legacy paths: `v0.6`.
