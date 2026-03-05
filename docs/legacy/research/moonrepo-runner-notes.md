# moonrepo Runner - Practical Notes for Please

## Key takeaways
- Rust provides strong ergonomics for command orchestration and buffered output handling.
- Developer UX matters: clear diagnostics and concise command output reduce friction.
- Graph-aware pipelines benefit from deterministic output ordering.

## Implications for Please v0.1
- Keep CLI surface small: run/list/graph/doctor/cache prune.
- Include deterministic execution summaries (executed, cache hit, dry-run).
- Keep graph output available in text and dot for debugging.
