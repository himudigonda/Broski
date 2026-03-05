# Install Guide

This guide covers installing `please` from GitHub Releases.

## Quick install (latest release)

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | bash
```

By default, installer channel is `latest`, which includes prereleases.

## Install stable-only channel

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_CHANNEL=stable bash
```

## Install a pinned version

```bash
curl -fsSL https://raw.githubusercontent.com/himudigonda/Please/main/install.sh | PLEASE_VERSION=v0.4.0-rc.1 bash
```

## Optional environment controls

- `PLEASE_REPO` (default: `himudigonda/Please`)
- `PLEASE_VERSION` (exact tag, with or without `v` prefix)
- `PLEASE_CHANNEL` (`latest` or `stable`; ignored if `PLEASE_VERSION` is set)
- `INSTALL_DIR` (default: `$HOME/.local/bin`)

## Verify install

```bash
please --version
```

If command is not found, add install dir to path:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Supported release targets

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`

## Troubleshooting

- `unsupported platform`: release binary for your OS/arch is not published yet.
- `checksum entry not found`: release assets are incomplete; check release page.
- `unable to resolve release tag`: GitHub API unavailable or no valid releases.
