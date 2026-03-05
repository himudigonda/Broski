# Please v0.4.0-rc.1 Release Notes

## Release type
- Tag: `v0.4.0-rc.1`
- Channel: release candidate prerelease
- Repository: `himudigonda/Please`

## Highlights
- Implicit task execution:
  - `please <task>`
  - `please <task> -- <args...>`
- DSL task descriptions surfaced in `please list`.
- Reserved command-name protection for task/alias identifiers.
- Variable engine for DSL:
  - static variables (`KEY = "value"`)
  - dynamic variables (`KEY = $(...)`)
  - strict interpolation (`{{ KEY }}`) with cycle/undefined diagnostics.
- `@requires` preflight checks for missing tool binaries.
- Secret redaction for interactive output and persisted logs.
- Native `--watch` task loop with input-scoped rerun and ignore filters.

## User-facing changes
- v0.4 DSL:
  - `version = "0.4"`
  - supports `@requires` and variable interpolation.
- Still compatible with:
  - DSL `version = "0.3"` (deprecated warning).
  - TOML `pleasefile` (deprecated warning).
- Watch behavior:
  - reruns selected target graph on relevant input changes.
  - ignores `.git`, `.please`, and declared output paths.

## Validation evidence
- `cargo fmt --all --check` passes.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `cargo test --workspace` passes.
- Root and `examples/**` `pleasefile`s migrated to `version = "0.4"`.
- Desktop benchmark lab (`/Users/himudigonda/Desktop/test`) runs with v0.4 DSL.

## Operational guidance
- Avoid nondeterministic dynamic variables (for example: `$(date)`) in graph tasks unless cache busting is intentional.
- Use explicit release pin for RC install:
  - `PLEASE_VERSION=v0.4.0-rc.1`

## Known limitations
- Dynamic-variable evaluation is optimized for referenced variables but still local-only.
- Remote cache backends remain out of scope.
- Embedded shebang/non-shell recipe bodies remain deferred.
- Windows remains out of current support scope.
