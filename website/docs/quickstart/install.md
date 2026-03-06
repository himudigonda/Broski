---
sidebar_position: 1
---

# Install

Use this page for verified install commands and post-install checks.

## Latest Stable

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

Expected output includes:

- downloaded release artifact
- checksum validation
- installed binary path

## Pinned Version

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.5.0 bash
```

Use pinned install for reproducible onboarding and CI bootstrap scripts.

## Verify

```bash
please --version
please --workspace . list
```

Expected behavior:

- `please --version` prints installed version
- `please --workspace . list` shows available tasks in current repo

## Smoke test

Create a temporary task file and run once:

```bash title="pleasefile"
version = "0.5"

hello:
    @mode interactive
    echo "please is installed"
```

```bash
please hello
```

Expected output:

- `please is installed`

## Supported Release Targets

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

## Next

- [30-Second Quickstart](./thirty-second-quickstart)
- [Your first pleasefile](./first-pleasefile)
- [Migration Playbook](../operations/migration)

Need help? Visit [https://himudigonda.me/please_docs/](https://himudigonda.me/please_docs/).
