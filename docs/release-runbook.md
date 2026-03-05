# Release Runbook: v0.5.0 (Stable)

## Scope
This runbook covers cutting and publishing `v0.5.0` for `himudigonda/Please`.

## Preconditions
- `develop` is clean and synced.
- Required CI jobs are green (Linux, macOS, Windows, showcase).
- You have permission to push tags and publish releases.

## Cut checklist
1. Sync and verify branch state.
   - `git checkout develop`
   - `git pull --ff-only`
   - `git status --short` (must be empty)
2. Run local quality gates.
   - `cargo fmt --all --check`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo test --workspace`
   - `cargo run -p please-cli -- --workspace . run ci`
3. Validate examples smoke path.
   - `cargo run -p please-cli -- --workspace . run examples_smoke`
4. Merge `develop` to `main` after green CI.
5. Tag release from `main`.
   - `git checkout main`
   - `git pull --ff-only`
   - `git tag -a v0.5.0 -m "Please v0.5.0"`
   - `git push origin v0.5.0`
6. Wait for `release.yml` completion.
7. Validate release artifacts:
   - `please-v0.5.0-x86_64-unknown-linux-gnu.tar.gz`
   - `please-v0.5.0-aarch64-apple-darwin.tar.gz`
   - `please-v0.5.0-x86_64-pc-windows-msvc.zip`
   - `SHA256SUMS.txt`

## Post-publish validation
1. Installer check (latest stable path):
   - `curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash`
   - `please --version`
2. Functional check:
   - `please --workspace . run ci --explain`
3. Showcase smoke:
   - `cd examples/showcase`
   - `../../target/debug/please --workspace . run build_ui`
   - `../../target/debug/please --workspace . run build_api`

## Rollback
1. If release must be yanked:
   - Delete GitHub release `v0.5.0`.
   - `git push origin :refs/tags/v0.5.0`
   - `git tag -d v0.5.0`
2. Fix forward with `v0.5.1` after patch and green CI.
