# Security Notes

## Secret environments

Use `@secret_env` for sensitive environment keys that tasks require at runtime:

```text
deploy:
    @mode interactive
    @secret_env API_TOKEN
    ./scripts/deploy.sh
```

Behavior in v0.4:
- secret values are hashed in fingerprints (never emitted in plain text),
- explain output reports generic changes (`cache miss: secret env changed`),
- interactive terminal output is redacted,
- persisted task stdout/stderr in cache metadata is redacted.

## Dynamic variable caution

Dynamic variables are supported:

```text
GIT_SHA = $(git rev-parse HEAD)
```

Avoid nondeterministic commands such as `$(date)` or `$(uuidgen)` in graph tasks unless repeated cache invalidation is intentional.
