# Migration Guide: `make` / `just` to `please`

`Please` is designed as a workflow replacement, not a syntax parser for existing `Makefile`/`justfile` formats.
The migration path is straightforward: map each command into an explicit task contract.

## Concept mapping
| Concept | make | just | please |
| --- | --- | --- | --- |
| Task definition | target rule | recipe | `[task.<name>]` |
| Dependencies | prerequisites | recipe dependencies | `deps = [...]` |
| Inputs | implicit, often mtime-based | implicit | `inputs = [...]` |
| Outputs | implicit | implicit | `outputs = [...]` |
| Command | recipe body | recipe body | `run = "..."` or `run = ["cmd", "arg"]` |
| Rebuild skip | timestamps | none by default | content fingerprint cache |

## Quick translation checklist
1. Pick each existing target/recipe and name a `task.<name>`.
2. Move command body into `run`.
3. Declare `deps` explicitly.
4. Declare `inputs` as glob/file patterns that affect output.
5. Declare `outputs` precisely.
6. Run `please --workspace . doctor` and then `please --workspace . run <task>`.

## Example: make -> please
`Makefile`:
```make
build: src/main.rs Cargo.toml
	cargo build --release
```

`pleasefile`:
```toml
[please]
version = "0.1"

[task.build]
inputs = ["src/main.rs", "Cargo.toml"]
outputs = ["target/release/app"]
run = "cargo build --release"
```

## Example: just -> please
`justfile`:
```just
fmt:
  cargo fmt --all --check

lint: fmt
  cargo clippy --workspace --all-targets --all-features -- -D warnings
```

`pleasefile`:
```toml
[please]
version = "0.1"

[task.fmt]
inputs = ["Cargo.toml", "crates/**/*.rs"]
outputs = [".please/stamps/fmt.ok"]
run = "mkdir -p .please/stamps && cargo fmt --all --check && printf 'ok\n' > .please/stamps/fmt.ok"

[task.lint]
deps = ["fmt"]
inputs = ["Cargo.toml", "crates/**/*.rs"]
outputs = [".please/stamps/lint.ok"]
run = "mkdir -p .please/stamps && cargo clippy --workspace --all-targets --all-features -- -D warnings && printf 'ok\n' > .please/stamps/lint.ok"
```

## Practical notes
- Prefer small, explicit output contracts. This improves cache correctness.
- Keep task commands shell-portable when possible.
- If a task mutates files outside declared `outputs`, behavior becomes non-portable and cache correctness is not guaranteed.

## Non-goals in v0.1
- No automatic conversion tool for Makefile/justfile yet.
- No remote shared cache yet.
