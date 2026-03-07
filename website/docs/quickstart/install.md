---
sidebar_position: 1
---

# Install

Use this page for verified install commands and a basic smoke test.

## Latest Stable

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | bash
```

## Pinned Version

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | BROSKI_VERSION=v0.6.1 bash
```

## Verify

```bash
broski --version
broski --workspace . list
```

## Smoke Test

```bash title="broskifile"
version = "0.5"

hello:
    echo "broski is installed"
```

```bash
broski hello
```

## Next

- [30-Second Quickstart](./thirty-second-quickstart)
- [Your first broskifile](./first-broskifile)
- [Migration Playbook](../operations/migration)
