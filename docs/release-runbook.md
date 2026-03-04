# Release Runbook: v0.2.0-beta.1

## Scope
This runbook covers cutting and publishing `v0.2.0-beta.1` for `himudigonda/Please`.

## Preconditions
- `main` is clean and synced.
- Required CI jobs are green (core + showcase).
- You have permission to push tags/publish releases.

## Cut checklist
1. Sync and verify branch state.
   - `git checkout main`
   - `git pull --ff-only`
   - `git status --short` (must be empty)
2. Run local quality gates.
   - `just ci`
   - `please --workspace . run ci`
3. Validate showcase proof.
   - `cd examples/showcase`
   - `../../target/debug/please --workspace . run package_container --explain`
4. Optional release dry-run.
   - `git tag v0.0.0-test-release`
   - `git push origin v0.0.0-test-release`
   - inspect release workflow and artifacts, then remove temp tag.
5. Tag beta release from `main`.
   - `git tag -a v0.2.0-beta.1 -m "Please v0.2.0-beta.1"`
   - `git push origin v0.2.0-beta.1`
6. Wait for `release.yml` completion.
7. Validate prerelease artifacts:
   - `please-v0.2.0-beta.1-x86_64-unknown-linux-gnu.tar.gz`
   - `please-v0.2.0-beta.1-aarch64-apple-darwin.tar.gz`
   - `SHA256SUMS.txt`

## Post-publish validation
1. Install check:
   - `PLEASE_VERSION=v0.2.0-beta.1 ./install.sh`
   - `please --version`
2. Functional check:
   - `please --workspace . run ci --explain`
3. Showcase smoke:
   - `examples/showcase` build/package tasks.

## Rollback / Yank
1. Delete prerelease in GitHub.
2. Delete remote tag:
   - `git push origin :refs/tags/v0.2.0-beta.1`
3. Delete local tag:
   - `git tag -d v0.2.0-beta.1`
4. Ship follow-up prerelease (`v0.2.0-beta.2`) after fixes.
