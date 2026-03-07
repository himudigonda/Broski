---
sidebar_position: 1
---

# Install

Use this page for verified install commands and post-install checks.

## Latest Stable

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | bash
```

Expected output includes:

- downloaded release artifact
- checksum validation
- installed binary path

## Pinned Version

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Broski/main/install.sh | BROSKI_VERSION=v0.5.2 bash
```

Use pinned install for reproducible onboarding and CI bootstrap scripts.

## Verify

```bash
broski --version
broski --workspace . list
```

Expected behavior:

- `broski --version` prints installed version
- `broski --workspace . list` shows available tasks in current repo

## Smoke test

Create a temporary task file and run once:

```bash title="broskifile"
version = "0.5"

hello:
    @mode interactive
    echo "broski is installed"
```

```bash
broski hello
```

Expected output:

- `broski is installed`

## Supported Release Targets

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

## Next

- [30-Second Quickstart](./thirty-second-quickstart)
- [Your first broskifile](./first-broskifile)
- [Migration Playbook](../operations/migration)

Need help? Visit [https://himudigonda.me/broski_docs/](https://himudigonda.me/broski_docs/).
