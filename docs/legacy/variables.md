# DSL Variables (v0.4)

`pleasefile` supports static and dynamic variables.

## Static variables

```text
version = "0.4"
PY = "backend/.venv/bin/python"

test:
    @in backend/**/*.py
    @out .please/stamps/test.ok
    @isolation off
    {{ PY }} -m pytest backend/tests -q
```

## Dynamic variables

```text
GIT_SHA = $(git rev-parse HEAD)
```

Dynamic variables are evaluated from workspace root.

## Interpolation

Use `{{ KEY }}` in:

- command lines
- `@in`, `@out`, `@dir`, `@env` values

## Safety rules

- Undefined variable reference is a parse error.
- Cyclic variable references are rejected.
- Dynamic commands have a timeout and fail with actionable diagnostics.

## Cache behavior

Resolved variable values used by a graph task are fingerprinted.
Changing a variable value invalidates cache as expected.

## Best practices

- Prefer deterministic dynamic vars (`git rev-parse HEAD`).
- Avoid nondeterministic vars (`date`, random UUIDs) for cached graph tasks unless frequent cache busting is intended.
- For secrets, use `@secret_env` rather than putting secret literals in variables.
