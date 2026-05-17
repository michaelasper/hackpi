#!/usr/bin/env bash
set -euo pipefail

REPO="${REPO:-https://github.com/anomalyco/hackpi.git}"
PACKAGE="${PACKAGE:-hackpi-tui}"
BIN_NAME="${BIN_NAME:-hackpi}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
TAG="${TAG:-}"  # optional: git tag for a specific release

print_step() { printf "\033[36m==>\033[0m \033[1m%s\033[0m\n" "$*"; }
print_ok()   { printf "\033[32m  ✓\033[0m %s\n" "$*"; }
print_err()  { printf "\033[31m  ✗\033[0m %s\n" "$*" >&2; }

TMPDIR=""
cleanup() { [ -n "$TMPDIR" ] && rm -rf "$TMPDIR"; }
trap cleanup EXIT

main() {
  echo ""
  printf "\033[1m\033[35m  ╔══════════════════════════╗\n"
  printf  "  ║   HackPI Installer v0.1  ║\n"
  printf  "  ╚══════════════════════════╝\n"
  echo ""

  # --- prerequisites ---
  print_step "Checking prerequisites..."

  if ! command -v rustc &>/dev/null; then
    print_err "Rust is not installed."
    echo "  Install it with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
  fi
  print_ok "rustc $(rustc --version | cut -d' ' -f2) found"

  if ! command -v cargo &>/dev/null; then
    print_err "Cargo is not installed."
    exit 1
  fi
  print_ok "cargo $(cargo --version | cut -d' ' -f2) found"

  # --- check for existing installation ---
  if command -v "$BIN_NAME" &>/dev/null; then
    existing_path="$(command -v "$BIN_NAME")"
    printf "  ⚠  Existing installation detected at: %s\n" "$existing_path"
    printf "  %s\n" "     It will be overwritten."
    echo ""
  fi

  # --- install ---
  print_step "Installing $BIN_NAME from $REPO..."

  TMPDIR="$(mktemp -d)"

  TAG_FLAG=""
  if [ -n "$TAG" ]; then
    TAG_FLAG="--tag $TAG"
  fi

  print_step "Building $BIN_NAME (this may take a few minutes)..."
  # shellcheck disable=SC2086
  cargo install $TAG_FLAG --git "$REPO" --root "$TMPDIR" "$PACKAGE" 2>&1 | sed 's/^/     /'

  # --- copy binary ---
  if ! mkdir -p "$INSTALL_DIR" 2>/dev/null; then
    print_err "Cannot write to $INSTALL_DIR (permission denied)."
    echo "  Retry with: sudo INSTALL_DIR=\"$INSTALL_DIR\" ./install.sh"
    echo "  Or set INSTALL_DIR to a user-writable path:"
    echo "    INSTALL_DIR=\"\$HOME/.local/bin\" ./install.sh"
    exit 1
  fi
  if ! cp -f "$TMPDIR/bin/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME" 2>/dev/null; then
    print_err "Cannot copy binary to $INSTALL_DIR (permission denied)."
    echo "  Retry with: sudo cp \"$TMPDIR/bin/$BIN_NAME\" \"$INSTALL_DIR/$BIN_NAME\""
    exit 1
  fi
  chmod +x "$INSTALL_DIR/$BIN_NAME"

  print_ok "$BIN_NAME installed to $INSTALL_DIR/$BIN_NAME"

  # --- verify ---
  if command -v "$BIN_NAME" &>/dev/null; then
    print_ok "Installation verified: $("$BIN_NAME" --help 2>/dev/null | head -1 || echo "$BIN_NAME installed")"
  else
    print_err "Binary not found in PATH after install."
    echo "  Make sure $INSTALL_DIR is in your PATH, or run:"
    echo "    export PATH=\"\$PATH:$INSTALL_DIR\""
    exit 1
  fi

  echo ""
  printf "\033[32m  ✓\033[0m \033[1mHackPI installed successfully!\033[0m\n"
  echo ""
  echo "  Quick start:"
  echo "    hackpi"
  echo ""
  echo "  Configuration (optional):"
  echo "    HACKPI_ENDPOINT  - API endpoint (default: http://localhost:11434/api/chat)"
  echo "    HACKPI_MODEL     - Model name (default: llama3.2)"
  echo "    HACKPI_MAX_TOKENS - Max tokens (default: 4096)"
  echo ""
}

main "$@"
