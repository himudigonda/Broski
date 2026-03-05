# Please v0.3.0-beta.3 Release Notes

## Release type
- Tag: `v0.3.0-beta.3`
- Channel: beta prerelease
- Repository: `himudigonda/Please`

## Highlights
- Hybrid executor with graph + interactive task modes.
- DSL-first `pleasefile` (`version = "0.3"`) with TOML fallback.
- Pass-through args support (`please run <task> -- ...`).
- Alias support across `run`, `graph`, and `list`.
- `.env` load support via `@load` and per-task env annotations.
- Expanded integration coverage for interactive bypass and passthrough cache deltas.

## User-facing changes
- New default file format:
  - `version = "0.3"`
  - `task: deps`
  - `@in`, `@out`, `@env`, `@secret_env`, `@dir`, `@mode`, `@isolation`
- Interactive mode runs in live workspace with inherited TTY and no cache writes.
- TOML `pleasefile` is still supported with a deprecation warning (planned removal target: v0.5).

## Validation evidence
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `cargo test --workspace` passes.
- `cargo run -p please-cli -- --workspace . run ci` passes.
- `cargo run -p please-cli -- --workspace . run examples_smoke` passes.

## Known limitations
- Remote cache backends are still out of scope.
- Embedded shebang/non-shell recipe bodies remain deferred.
- Windows remains out of current support scope.
