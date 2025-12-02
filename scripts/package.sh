#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package.sh [--target TRIPLE] [--skip-checks]

Build a release binary and assemble a distributable tarball under dist/.

Options:
  --target TRIPLE  Build for the specified target (default: host triple)
  --skip-checks    Skip fmt/clippy and run only cargo build --release
  -h, --help       Show this help
EOF
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${ROOT}/dist"
PKG_SRC="${ROOT}/packaging"
NAME="hyprsets"
TARGET=""
SKIP_CHECKS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      if [[ $# -lt 2 ]]; then
        echo "error: --target requires a value" >&2
        exit 1
      fi
      TARGET="$2"
      shift
      ;;
    --skip-checks)
      SKIP_CHECKS=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

if [[ -z "${TARGET}" ]]; then
  TARGET="$(rustc -vV | sed -n 's/^host: //p')"
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 is required to parse cargo metadata" >&2
  exit 1
fi

VERSION="$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; data=json.load(sys.stdin); print(next(p["version"] for p in data["packages"] if p["name"]=="hyprsets"))')"

echo "==> package ${NAME} v${VERSION} for target ${TARGET}"

if [[ "${SKIP_CHECKS}" -ne 1 ]]; then
  echo "==> running cargo fmt"
  cargo fmt
  echo "==> running cargo clippy"
  cargo clippy --target "${TARGET}" -- -D warnings
fi

echo "==> building release binary"
cargo build --release --target "${TARGET}"

BIN_PATH="${ROOT}/target/${TARGET}/release/${NAME}"
if [[ ! -x "${BIN_PATH}" ]]; then
  echo "error: built binary not found at ${BIN_PATH}" >&2
  exit 1
fi

PKG_DIR="${DIST_DIR}/${NAME}-${VERSION}-${TARGET}"
TARBALL="${DIST_DIR}/${NAME}-${VERSION}-${TARGET}.tar.gz"
echo "==> staging files in ${PKG_DIR}"
rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}/bin" "${PKG_DIR}/share/applications" "${PKG_DIR}/share/hyprsets"

cp "${BIN_PATH}" "${PKG_DIR}/bin/"
cp "${PKG_SRC}/hyprsets.desktop" "${PKG_DIR}/share/applications/"
cp "${PKG_SRC}/sample-worksets.toml" "${PKG_DIR}/share/hyprsets/"
cp "${PKG_SRC}/install.sh" "${PKG_DIR}/"
cp "${PKG_SRC}/README.dist.md" "${PKG_DIR}/"

echo "==> writing checksums"
(
  cd "${PKG_DIR}"
  sha256sum bin/hyprsets share/applications/hyprsets.desktop share/hyprsets/sample-worksets.toml install.sh README.dist.md > CHECKSUMS.txt
)

echo "==> creating tarball ${TARBALL}"
mkdir -p "${DIST_DIR}"
tar -C "${DIST_DIR}" -czf "${TARBALL}" "$(basename "${PKG_DIR}")"
(cd "${DIST_DIR}" && sha256sum "$(basename "${TARBALL}")" > "$(basename "${TARBALL}").sha256")

echo "done."
echo "  package dir : ${PKG_DIR}"
echo "  tarball     : ${TARBALL}"
