# Please v0.5.0 Release Notes

## Release type
- Tag: `v0.5.0`
- Channel: stable (pre-1.0)
- Repository: `himudigonda/Please`

## Highlights
- Public-stable DSL `version = "0.5"`.
- First-class task parameters (`task [arg] [arg="default"]:`).
- Modular task composition with `@import`.
- Decorators: `@private`, `@confirm`.
- Built-in interpolation helpers: `os()`, `arch()`, `env()`.
- Shebang task bodies for embedded polyglot scripts.
- Improved diagnostics via `miette` source spans.
- Cross-platform shell resolution with Windows support (`pwsh` -> `cmd`).

## Compatibility
- DSL `version = "0.3"` and `version = "0.4"` remain supported with warnings.
- TOML `pleasefile` remains supported with warning.
- Removal target for legacy formats: `v0.6`.

## Release artifacts
- `please-v0.5.0-x86_64-unknown-linux-gnu.tar.gz`
- `please-v0.5.0-aarch64-apple-darwin.tar.gz`
- `please-v0.5.0-x86_64-pc-windows-msvc.zip`
- `SHA256SUMS.txt`

## Validation evidence
- `cargo fmt --all --check` passes.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `cargo test --workspace` passes.
- `please --workspace . run ci` passes.
- Root + `examples/**` migrated to DSL `version = "0.5"`.
- Desktop benchmark lab updated for v0.5.

## Installer behavior
- Unpinned installer resolves latest stable release.
- Pinned install remains supported:
  - `PLEASE_VERSION=v0.5.0`
