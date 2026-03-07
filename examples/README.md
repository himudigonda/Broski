# Examples

Runnable `broskifile` examples focused on practical onboarding.

## Start Here

- `minimal`: smallest graph-mode task with `@in` + `@out`.
- `basic`: simple multi-step file pipeline.
- `python-cli`: normal test/build workflow with cache reuse.

## Broader Stack Examples

- `go-http`: Go service build + test.
- `node-web`: Node service build + test.
- `polyglot`: mixed-language pipeline.
- `showcase`: heavier demo (not the first onboarding stop).

## Run all smoke tasks

From repository root:

```bash
broski --workspace . run examples_smoke --explain
```

## Run one example directly

```bash
cargo run -p broski-cli -- --workspace examples/python-cli run ci --explain
```
