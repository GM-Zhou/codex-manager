#!/usr/bin/env sh
set -eu

BIN_NAME="codexm"
REPO="${CODEXM_REPO:-}"
VERSION="${CODEXM_VERSION:-latest}"
INSTALL_DIR="${CODEXM_INSTALL_DIR:-$HOME/.local/bin}"

usage() {
  cat <<'EOF'
Install codexm from GitHub Releases.

Usage:
  install.sh --repo <owner/repo> [--version <tag|latest>] [--install-dir <dir>]

Examples:
  curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.sh | sh -s -- --repo <owner/repo>
  curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.sh | sh -s -- --repo <owner/repo> --version v0.1.0

Options:
  --repo         GitHub repository in owner/repo format (required if CODEXM_REPO is unset)
  --version      Release tag, e.g. v0.1.0 (default: latest)
  --install-dir  Target install directory (default: ~/.local/bin)
  -h, --help     Show this help
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo)
      REPO="${2:-}"
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --install-dir)
      INSTALL_DIR="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [ -z "$REPO" ]; then
  echo "Error: --repo is required (or set CODEXM_REPO)." >&2
  usage >&2
  exit 1
fi

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  darwin|linux) ;;
  *)
    echo "Error: unsupported OS: $OS" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="amd64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *)
    echo "Error: unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

if [ "$VERSION" = "latest" ]; then
  RELEASE_PATH="releases/latest/download"
else
  RELEASE_PATH="releases/download/$VERSION"
fi

ASSET_BASE="${BIN_NAME}-${OS}-${ARCH}"
TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

download() {
  url="$1"
  target="$2"
  if curl -fL --connect-timeout 10 --retry 2 --retry-delay 1 "$url" -o "$target"; then
    return 0
  fi
  return 1
}

BASE_URL="https://github.com/$REPO/$RELEASE_PATH"
BIN_FILE="$TMP_DIR/$ASSET_BASE"
ARCHIVE_FILE="$TMP_DIR/$ASSET_BASE.tar.gz"

echo "Installing $BIN_NAME from $REPO ($VERSION) for ${OS}-${ARCH}..."

if download "$BASE_URL/$ASSET_BASE" "$BIN_FILE"; then
  :
elif download "$BASE_URL/$ASSET_BASE.tar.gz" "$ARCHIVE_FILE"; then
  tar -xzf "$ARCHIVE_FILE" -C "$TMP_DIR"
  if [ -f "$TMP_DIR/$ASSET_BASE" ]; then
    BIN_FILE="$TMP_DIR/$ASSET_BASE"
  elif [ -f "$TMP_DIR/$BIN_NAME" ]; then
    BIN_FILE="$TMP_DIR/$BIN_NAME"
  else
    echo "Error: archive does not contain $ASSET_BASE or $BIN_NAME." >&2
    exit 1
  fi
else
  echo "Error: failed to download asset from:" >&2
  echo "  $BASE_URL/$ASSET_BASE" >&2
  echo "  $BASE_URL/$ASSET_BASE.tar.gz" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
chmod +x "$BIN_FILE"
TARGET_PATH="$INSTALL_DIR/$BIN_NAME"
cp "$BIN_FILE" "$TARGET_PATH"

echo "Installed to: $TARGET_PATH"
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo "Run: $BIN_NAME --help"
    ;;
  *)
    echo "Warning: $INSTALL_DIR is not in PATH."
    echo "Add this to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
