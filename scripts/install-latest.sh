#!/usr/bin/env bash
set -euo pipefail

# Install the latest hyprsets release for the default target.
# Customize with env vars:
#   TARGET=x86_64-unknown-linux-gnu  # download target triple (wins over TARGET_ALIAS)
#   TARGET_ALIAS=linux-x86_64        # friendlier alias mapped to a target triple
#   INSTALL_ARGS="--user"             # passed to install.sh (e.g., "--user --no-config")

resolve_target() {
  local alias="${TARGET_ALIAS:-linux-x86_64}"
  case "${alias}" in
    linux-x86_64|x86_64-linux|linux-amd64)
      echo "x86_64-unknown-linux-gnu" ;;
    linux-aarch64|aarch64-linux|linux-arm64)
      echo "aarch64-unknown-linux-gnu" ;;
    *)
      echo "${alias}" ;;
  esac
}

TARGET="${TARGET:-$(resolve_target)}"
INSTALL_ARGS=${INSTALL_ARGS:---user}
REPO="agata/hyprsets"

echo "==> Detecting latest release tag" >&2
TAG="$(basename "$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")")"
ARCHIVE="hyprsets-${TAG#v}-${TARGET}.tar.gz"

echo "==> Downloading ${ARCHIVE}" >&2
curl -fLO "https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE}"
curl -fLO "https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE}.sha256"

echo "==> Verifying checksum" >&2
# Normalize absolute paths inside the sha256 file (older releases used absolute paths)
sha256sum -c <(sed "s#  .*#  ${ARCHIVE}#" "${ARCHIVE}.sha256")

echo "==> Extracting" >&2
tar -xf "${ARCHIVE}"
cd "hyprsets-${TAG#v}-${TARGET}"

echo "==> Installing (install.sh ${INSTALL_ARGS})" >&2
./install.sh ${INSTALL_ARGS}
