# Staging Performance Checks

## Manual 2GB Reflink Check (outside CI)

```bash
mkdir -p /tmp/please-perf/workspace/data
python3 - <<'PY'
from pathlib import Path
p = Path('/tmp/please-perf/workspace/data/large.bin')
chunk = b'Z' * (1024 * 1024)
with p.open('wb') as f:
    for _ in range(2048):
        f.write(chunk)
PY

# Run any task that triggers staging from this workspace and compare elapsed times.
# For example, run twice and compare cold timings:
# cargo run -p please-cli -- --workspace /tmp/please-perf/workspace run <task>
```

Expected result on reflink-capable filesystems: staging time should be near constant and much lower
than a deep byte-for-byte copy.
