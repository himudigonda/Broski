# Please Showcase (React + Rust + Docker)

This directory proves real-world orchestration with Please:
- React dashboard frontend (`frontend`)
- Axum backend serving APIs + static UI (`backend`)
- Single runtime Docker image (`Dockerfile`)

## Run locally
From `examples/showcase`:

```bash
../../target/debug/please --workspace . run build_ui
../../target/debug/please --workspace . run build_api
PORT=8080 STATIC_DIR=frontend/dist backend/target/release/showcase-server
```

## Package container
```bash
../../target/debug/please --workspace . run package_container --explain
```

## Prove cache selectivity
```bash
../../target/debug/please --workspace . run prove_cache
```

The proof script runs cold/warm/mutation scenarios and writes `artifacts/prove-cache.log`.
