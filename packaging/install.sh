#!/usr/bin/env bash
set -euo pipefail

print_help() {
  cat <<'EOF'
Usage: ./install.sh [--user] [--prefix PATH] [--no-config] [--force]

Install the packaged hyprsets binary and desktop entry.

Options:
  --user        Install into ~/.local (default prefix: /usr/local)
  --prefix PATH Override install prefix (implies system install)
  --no-config   Do not install sample config
  --force       Overwrite existing config if present
  -h, --help    Show this help
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX=${PREFIX:-/usr/local}
INSTALL_CONFIG=1
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user)
      PREFIX="${HOME}/.local"
      ;;
    --prefix)
      if [[ $# -lt 2 ]]; then
        echo "error: --prefix requires a path" >&2
        exit 1
      fi
      PREFIX="$2"
      shift
      ;;
    --no-config)
      INSTALL_CONFIG=0
      ;;
    --force)
      FORCE=1
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      print_help
      exit 1
      ;;
  esac
  shift
done

BIN_DIR="${PREFIX}/bin"
APP_DIR="${PREFIX}/share/applications"

echo "Install prefix: ${PREFIX}"
echo "  bin   -> ${BIN_DIR}"
echo "  desktop -> ${APP_DIR}"

install -d "${BIN_DIR}" "${APP_DIR}"
install -m 755 "${SCRIPT_DIR}/bin/hyprsets" "${BIN_DIR}/hyprsets"

DESKTOP_SRC="${SCRIPT_DIR}/share/applications/hyprsets.desktop"
DESKTOP_TMP="$(mktemp)"
DESKTOP_PATH="${APP_DIR}/hyprsets.desktop"
sed "s|__HYPRSETS_BIN__|${BIN_DIR}/hyprsets|g" "${DESKTOP_SRC}" > "${DESKTOP_TMP}"
if [[ -e "${DESKTOP_PATH}" ]]; then
  echo "replacing existing desktop entry at ${DESKTOP_PATH}"
  rm -f "${DESKTOP_PATH}"
fi
install -m 644 "${DESKTOP_TMP}" "${DESKTOP_PATH}"
rm -f "${DESKTOP_TMP}"

if [[ "${INSTALL_CONFIG}" -eq 1 ]]; then
  CFG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/hyprsets"
  CFG_PATH="${CFG_DIR}/hyprsets.toml"
  install -d "${CFG_DIR}"
  if [[ -e "${CFG_PATH}" && "${FORCE}" -ne 1 ]]; then
    echo "config already exists at ${CFG_PATH}; keeping existing file (use --force to overwrite)"
  else
    install -m 644 "${SCRIPT_DIR}/share/hyprsets/sample-worksets.toml" "${CFG_PATH}"
    echo "sample config written to ${CFG_PATH}"
  fi
fi

echo "Install finished. You may need to run 'update-desktop-database' for desktop entries to be picked up."
