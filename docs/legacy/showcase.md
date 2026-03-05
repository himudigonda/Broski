# Showcase: React + Rust + Docker via Please

`examples/showcase` is the end-to-end proof-of-value project.

## Components
- `frontend`: Vite + React + TypeScript dashboard.
- `backend`: Axum server exposing API endpoints and static file hosting.
- `Dockerfile`: single-runtime image packaging prebuilt artifacts.
- `pleasefile`: orchestrates build graph and cache behavior.

## Main tasks
- `build_ui`: `npm ci && npm run build` -> `frontend/dist`
- `build_api`: `cargo build --release` -> `backend/target/release/showcase-server`
- `package_container`: docker build + docker save -> `artifacts/showcase-image.tar`
- `prove_cache`: runs mutation scenarios and logs explain output

## Run
```bash
cd examples/showcase
../../target/debug/please --workspace . run package_container --explain
```

## Smoke test without Docker run
```bash
PORT=18080 STATIC_DIR=frontend/dist backend/target/release/showcase-server
curl -fsS http://127.0.0.1:18080/api/health
```

## Cache proof
```bash
../../target/debug/please --workspace . run prove_cache
cat artifacts/prove-cache.log
```

Expected behavior:
- first run executes required tasks
- warm run favors cache hits
- frontend-only mutation invalidates UI path only
- backend-only mutation invalidates API path only
