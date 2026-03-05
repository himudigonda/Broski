# Build Systems a la Carte - Notes for Please

## Key takeaways
- Build engines are composed from scheduling policy + rebuild policy + storage model.
- Timestamp-based invalidation is fast but can be incorrect after branch switches and clock drift.
- Content-based invalidation gives stronger correctness and deterministic rebuild behavior.
- DAG scheduling should expose maximal parallelism while preserving dependency order.

## Implications for Please v0.1
- Rebuild policy: content hash (BLAKE3), never mtime.
- Scheduler: deterministic topological layers with parallel execution inside each layer.
- Storage: local CAS + SQLite metadata as baseline.

## Deferred to post-v0.1
- Speculative execution and dynamic dependencies.
- Remote execution protocol.
