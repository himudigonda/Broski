# DSL v0.5 Reference

## File header
```text
version = "0.5"
```

## Top-level declarations
- Variables:
  - `KEY = "value"`
  - `KEY = $(command)`
- Alias:
  - `alias short = target`
- Env file load:
  - `@load .env`
- Import:
  - `@import path/to/pleasefile`

## Task definition
```text
task_name [param_a] [param_b="default"]: dep_a dep_b
    <annotations>
    <command lines>
```

## Annotations
- `@in <glob...>`
- `@out <path...>`
- `@env KEY=value` and `@env KEY`
- `@secret_env KEY`
- `@dir path/to/subdir`
- `@mode graph|interactive`
- `@isolation strict|best_effort|off`
- `@requires tool_a tool_b`
- `@private`
- `@confirm "message"`

## Interpolation
- Variables: `{{ KEY }}`
- Params: `{{ param_name }}`
- Built-ins:
  - `{{ os() }}`
  - `{{ arch() }}`
  - `{{ env("KEY", "default") }}`

## Shebang bodies
If the task body starts with `#!`, Please writes a temporary script and executes it directly:

```text
lint_py:
    #!/usr/bin/env python3
    print("hello")
```

## Compatibility in v0.5
- DSL `0.3` and `0.4`: supported with deprecation warning.
- TOML `pleasefile`: supported with deprecation warning.
- Removal target: `v0.6`.
