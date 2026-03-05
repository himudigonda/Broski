# Release Runbook: v0.3.0-beta.2

## Scope
This runbook covers cutting and publishing `v0.3.0-beta.2` for `himudigonda/Please`.

## Preconditions
- `develop` is clean and synced.
- Required CI jobs are green (core + showcase).
- You have permission to push tags/publish releases.

## Cut checklist
1. Sync and verify branch state.
   - `git checkout develop`
   - `git pull --ff-only`
   - `git status --short` (must be empty)
2. Run local quality gates.
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo test --workspace`
   - `cargo run -p please-cli -- --workspace . run ci`
3. Validate example smoke path.
   - `cargo run -p please-cli -- --workspace . run examples_smoke`
4. Tag release from `develop`.
   - `git tag -a v0.3.0-beta.2 -m "Please v0.3.0-beta.2"`
   - `git push origin v0.3.0-beta.2`
5. Wait for `release.yml` completion.
6. Validate release artifacts:
   - `please-v0.3.0-beta.2-x86_64-unknown-linux-gnu.tar.gz`
   - `please-v0.3.0-beta.2-aarch64-apple-darwin.tar.gz`
   - `SHA256SUMS.txt`

## Post-publish validation
1. Install check (explicit beta pin):
   - `curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.3.0-beta.2 bash`
   - `please --version`
2. Functional check:
   - `please --workspace . run ci --explain`
3. Showcase smoke:
   - `cd examples/showcase`
   - `../../target/debug/please --workspace . run build_ui`
   - `../../target/debug/please --workspace . run build_api`

## Rollback / Yank
1. Delete release in GitHub.
2. Delete remote tag:
   - `git push origin :refs/tags/v0.3.0-beta.2`
3. Delete local tag:
   - `git tag -d v0.3.0-beta.2`
4. Ship follow-up beta (`v0.3.0-beta.3`) after fixes.
