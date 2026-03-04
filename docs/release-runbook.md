# Release Runbook: v0.1.0-alpha.1

## Scope
This runbook covers cutting and publishing `v0.1.0-alpha.1` for `himudigonda/Please`.

## Preconditions
- You are on `main` and the tree is clean.
- GitHub Actions is enabled for this repository.
- You have permission to push tags and publish releases.

## Cut checklist
1. Sync and verify branch state.
   - `git checkout main`
   - `git pull --ff-only`
   - `git status --short` (must be empty)
2. Run full quality gate.
   - `just ci`
3. Optional release pipeline dry-run.
   - Create temporary tag: `git tag v0.0.0-test-release`
   - Push: `git push origin v0.0.0-test-release`
   - Verify workflow and draft release artifacts.
   - Delete temp release and tag after validation.
4. Create the alpha tag from `main`.
   - `git tag -a v0.1.0-alpha.1 -m "Please v0.1.0-alpha.1"`
   - `git push origin v0.1.0-alpha.1`
5. Wait for `release.yml` workflow completion.
6. Validate draft release contents.
   - Artifacts:
     - `please-v0.1.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz`
     - `please-v0.1.0-alpha.1-aarch64-apple-darwin.tar.gz`
     - `SHA256SUMS.txt`
   - Confirm checksum file contains entries for both artifacts.
7. Publish the draft prerelease manually in GitHub UI.

## Post-publish validation
1. Install from published release with installer script.
2. Verify binary reports version:
   - `please --version`
3. Run smoke commands in a sample project:
   - `please list`
   - `please run ci`

## Rollback / Yank procedure
1. In GitHub Releases, mark release as draft or delete it.
2. Delete remote tag:
   - `git push origin :refs/tags/v0.1.0-alpha.1`
3. Delete local tag:
   - `git tag -d v0.1.0-alpha.1`
4. Communicate rollback reason in issue/PR notes.

## Hotfix alpha (`v0.1.0-alpha.2`)
1. Land fix commits on `main` with green `just ci`.
2. Tag new prerelease:
   - `git tag -a v0.1.0-alpha.2 -m "Please v0.1.0-alpha.2"`
   - `git push origin v0.1.0-alpha.2`
3. Publish new draft prerelease after artifact validation.
4. Keep `v0.1.0-alpha.1` notes for historical traceability.
