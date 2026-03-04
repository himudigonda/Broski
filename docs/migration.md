# Migration Guide: `make` / `just` to `please`

`Please` replaces workflow orchestration semantics, not Make/Just file syntax.

## Important
There is intentionally **no automatic importer** in v0.2.0-beta.1.
Manual migration keeps `inputs`/`outputs` explicit and preserves deterministic caching and ACID semantics.

## Concept mapping
| Concept | make | just | please |
| --- | --- | --- | --- |
| Task definition | target | recipe | `[task.<name>]` |
| Dependencies | prerequisites | dependencies | `deps = []` |
| Inputs | implicit/mtime | implicit | `inputs = []` |
| Outputs | implicit | implicit | `outputs = []` |
| Command body | recipe | recipe | `run = "..."` or `run = ["cmd", "arg"]` |
| Rebuild logic | timestamps | rerun by default | content fingerprint + cache |

## Translation checklist
1. Create one `task.<name>` per legacy target/recipe.
2. Move command body into `run`.
3. Add `deps` explicitly.
4. Add all cache-relevant source/config patterns to `inputs`.
5. Add concrete artifacts/stamps to `outputs`.
6. Validate via:
   - `please --workspace . doctor`
   - `please --workspace . run <task> --explain`

## Example: make -> please
```make
build: src/main.rs Cargo.toml
	cargo build --release
```

```toml
[please]
version = "0.2"

[task.build]
inputs = ["src/main.rs", "Cargo.toml"]
outputs = ["target/release/app"]
run = "cargo build --release"
```

## Example: just -> please
```just
lint:
  cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```toml
[please]
version = "0.2"

[task.lint]
inputs = ["Cargo.toml", "crates/**/*.rs"]
outputs = [".please/stamps/lint.ok"]
run = "mkdir -p .please/stamps && cargo clippy --workspace --all-targets --all-features -- -D warnings && printf 'ok\\n' > .please/stamps/lint.ok"
```

## Debugging misses
Use explain mode to identify exactly what changed:
```bash
please --workspace . run lint --explain
```
