#!/usr/bin/env bash
set -euo pipefail

REPO="${PLEASE_REPO:-himudigonda/Please}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${PLEASE_VERSION:-}"
temp_dir=""

fail() {
  echo "install.sh: $*" >&2
  exit 1
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "required command not found: $1"
  fi
}

resolve_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}-${arch}" in
    Linux-x86_64|Linux-amd64)
      echo "x86_64-unknown-linux-gnu"
      ;;
    Darwin-arm64|Darwin-aarch64)
      echo "aarch64-apple-darwin"
      ;;
    *)
      fail "unsupported platform ${os}/${arch}; supported: Linux x86_64 and macOS arm64"
      ;;
  esac
}

resolve_tag() {
  if [ -n "$VERSION" ]; then
    if [[ "$VERSION" == v* ]]; then
      echo "$VERSION"
    else
      echo "v$VERSION"
    fi
    return
  fi

  local api_latest api_all response tag
  api_latest="https://api.github.com/repos/${REPO}/releases/latest"
  api_all="https://api.github.com/repos/${REPO}/releases?per_page=20"

  response="$(curl -fsSL "$api_latest" 2>/dev/null || true)"
  tag="$(printf '%s\n' "$response" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"

  if [ -z "$tag" ]; then
    response="$(curl -fsSL "$api_all")"
    tag="$(
      python3 -c '
import json, os, sys

allow_prerelease = os.environ.get("PLEASE_ALLOW_PRERELEASE", "").lower() in {"1", "true", "yes"}

try:
    releases = json.load(sys.stdin)
except Exception:
    print("")
    raise SystemExit(0)

candidate = ""
for rel in releases:
    if rel.get("draft"):
        continue
    if not rel.get("prerelease", False):
        candidate = rel.get("tag_name", "")
        break

if not candidate and allow_prerelease:
    for rel in releases:
        if rel.get("draft"):
            continue
        candidate = rel.get("tag_name", "")
        if candidate:
            break

print(candidate)
' <<<"$response"
    )"
  fi

  if [ -z "$tag" ]; then
    fail "unable to resolve latest stable release tag from GitHub API; set PLEASE_VERSION explicitly (or set PLEASE_ALLOW_PRERELEASE=1)"
  fi

  echo "$tag"
}

verify_checksum() {
  local checksum_file asset_file
  checksum_file="$1"
  asset_file="$2"

  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$asset_file")" && sha256sum --check "$(basename "$checksum_file")")
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    (cd "$(dirname "$asset_file")" && shasum -a 256 -c "$(basename "$checksum_file")")
    return
  fi

  fail "no checksum tool found; install sha256sum or shasum"
}

cleanup() {
  if [ -n "${temp_dir:-}" ] && [ -d "${temp_dir:-}" ]; then
    rm -rf "$temp_dir"
  fi
}

main() {
  require_cmd curl
  require_cmd tar
  require_cmd mktemp

  local target tag asset base_url temp_dir checksum_file
  target="$(resolve_target)"
  tag="$(resolve_tag)"
  asset="please-${tag}-${target}.tar.gz"
  base_url="https://github.com/${REPO}/releases/download/${tag}"

  temp_dir="$(mktemp -d)"
  trap cleanup EXIT

  echo "Downloading ${asset} from ${REPO} (${tag})"
  curl -fLsS "${base_url}/${asset}" -o "${temp_dir}/${asset}" || fail "failed to download ${asset}"
  curl -fLsS "${base_url}/SHA256SUMS.txt" -o "${temp_dir}/SHA256SUMS.txt" || fail "failed to download SHA256SUMS.txt"

  checksum_file="${temp_dir}/SHA256SUMS-${tag}.txt"
  grep "  ${asset}$" "${temp_dir}/SHA256SUMS.txt" > "$checksum_file" || fail "checksum entry not found for ${asset}"

  verify_checksum "$checksum_file" "${temp_dir}/${asset}"

  tar -xzf "${temp_dir}/${asset}" -C "$temp_dir"
  [ -f "${temp_dir}/please" ] || fail "archive missing please binary"

  mkdir -p "$INSTALL_DIR"
  install -m 0755 "${temp_dir}/please" "${INSTALL_DIR}/please"

  echo "Installed please to ${INSTALL_DIR}/please"
  if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
    echo "Add this to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  fi
}

main "$@"
