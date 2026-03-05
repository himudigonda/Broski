# Watch Mode

Run tasks continuously with file-triggered reruns:

```bash
please --workspace . run test --watch
# or implicit form
please --workspace . test --watch
```

## Behavior

- Watches resolved input roots for the selected target graph.
- Re-runs target graph on relevant input changes.
- Applies normal cache behavior for each cycle.

## Loop prevention

Watch mode ignores:

- `.git/**`
- `.please/**`
- declared task outputs (`@out`)

This prevents self-trigger loops from cache/stamp/output writes.

## Interactive task note

If target mode is interactive, Please prints:

`info: task '<name>' is interactive; internal watchers may conflict with --watch`

This is expected for tools like Vite/Nodemon that already watch files internally.

## Ctrl+C handling

Watch loop exits cleanly on interrupt and attempts graceful child shutdown.

## Recommended usage

- Use `--watch` mainly with graph tasks (tests, codegen, local CI loops).
- For long-running dev servers that already watch files, run without `--watch`.
