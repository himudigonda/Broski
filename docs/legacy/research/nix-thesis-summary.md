# Nix Thesis - Notes for Please

## Key takeaways
- Derivations become reproducible when all inputs are explicit and hashed.
- Output paths should be tied to input identity to avoid hidden mutable state.
- Purity has practical tradeoffs: strict isolation can increase setup complexity.

## Implications for Please v0.1
- Task fingerprint includes run payload, env map, inputs, and output declarations.
- Successful task outputs are persisted in a content-addressed store.
- Isolation policy is strict on Linux and best-effort on macOS, with explicit behavior docs.

## Deferred to post-v0.1
- Distributed binary cache trust model.
- Fully pure dependency closure (toolchain pinning).
