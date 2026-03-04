# Cache Telemetry (`--explain`)

## Purpose
Cache telemetry explains why a task executed instead of hitting cache.

## How it works
1. Fingerprinting now emits:
   - aggregate fingerprint hash
   - manifest map (`BTreeMap<String, String>`) of component digests
2. Execution records persist the manifest in SQLite (`manifest_json`).
3. On a miss with `--explain`, Please compares the current manifest with the latest prior execution for that task.
4. Differences are rendered as stable reason lines.

## Manifest key categories
- `meta:*`
- `task:*`
- `env:*`
- `input_pattern:*`
- `input:*`
- `output:*`

## Example output
```text
executed: build_api
explain build_api:
- cache miss: input changed: backend/src/main.rs
```

## Bypass output
```text
executed: build_api
explain build_api:
- cache bypass: --no-cache supplied
```

## Notes
- `--explain` is opt-in to keep standard output concise.
- Diagnostics are deterministic and sorted for stable CI assertions.
