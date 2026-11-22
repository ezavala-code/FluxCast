#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${FLUXCAST_INSTALL_PREFIX:-$HOME/.local}"
SYSTEMD_USER_DIR="${FLUXCAST_SYSTEMD_USER_DIR:-$HOME/.config/systemd/user}"
APPS_DIR="${FLUXCAST_APPS_DIR:-$PREFIX/share/applications}"
ICONS_DIR="${FLUXCAST_ICONS_DIR:-$PREFIX/share/icons/hicolor/128x128/apps}"
ENABLE_SERVICE=1

for arg in "$@"; do
  case "$arg" in
    --no-systemd) ENABLE_SERVICE=0 ;;
    --help|-h)
      echo "Uso: $0 [--no-systemd]"
      echo "Instala los binarios fluxcast y fluxcast-desktop, la entrada .desktop y la unidad systemd --user."
      exit 0
      ;;
  esac
done

if [[ ! -x "$ROOT_DIR/target/release/fluxcast" || ! -x "$ROOT_DIR/target/release/fluxcast-desktop" ]]; then
  echo "==> Compilando binarios release de FluxCast..."
  (cd "$ROOT_DIR" && pnpm --filter @fluxcast/desktop tauri build --no-bundle)
fi

mkdir -p "$PREFIX/bin" "$SYSTEMD_USER_DIR" "$APPS_DIR" "$ICONS_DIR"

install -m 0755 "$ROOT_DIR/target/release/fluxcast" "$PREFIX/bin/fluxcast"
install -m 0755 "$ROOT_DIR/target/release/fluxcast-desktop" "$PREFIX/bin/fluxcast-desktop"
install -m 0644 "$ROOT_DIR/apps/desktop/src-tauri/icons/icon.png" "$ICONS_DIR/fluxcast.png"
install -m 0644 "$ROOT_DIR/scripts/fluxcast.service" "$SYSTEMD_USER_DIR/fluxcast.service"

cat > "$APPS_DIR/dev.fluxcast.desktop.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=FluxCast
GenericName=Launcher y Portapapeles E2EE
Comment=Launcher nativo para Fedora KDE Plasma con portapapeles Wayland, snippets y sync E2EE
Exec=$PREFIX/bin/fluxcast
Icon=fluxcast
Terminal=false
Categories=Utility;Development;
Keywords=launcher;clipboard;snippets;raycast;kde;wayland;
StartupNotify=false
EOF

if [[ "$ENABLE_SERVICE" -eq 1 ]] && command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload || true
  systemctl --user enable fluxcast.service || true
  echo "==> Unidad systemd --user instalada y habilitada (fluxcast.service)."
fi

echo "OK · FluxCast instalado en $PREFIX/bin/fluxcast"
