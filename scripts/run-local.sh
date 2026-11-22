#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ ! -x "$ROOT_DIR/target/release/fluxcast" || ! -x "$ROOT_DIR/target/release/fluxcast-desktop" ]]; then
  echo "==> Compilando FluxCast Desktop para Fedora KDE..."
  pnpm --filter @fluxcast/desktop tauri build --no-bundle
fi

if ! ss -ltn 2>/dev/null | grep -q ':8787 '; then
  echo "==> Iniciando FluxCast Sync Relay Server en http://127.0.0.1:8787 ..."
  cargo build -p fluxcast-sync-server
  FLUXCAST_BIND="127.0.0.1:8787" "$ROOT_DIR/target/debug/fluxcast-sync-server" >/tmp/fluxcast-sync-server.log 2>&1 &
  SERVER_PID=$!
  trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
  sleep 0.5
else
  echo "==> FluxCast Sync Relay Server ya está escuchando en el puerto 8787."
fi

echo "==> Iniciando FluxCast en tu sesión de Fedora KDE Plasma..."
echo "    • Atajo global KDE: Alt+Space (o ejecuta './target/release/fluxcast toggle')"
echo "    • Pestañas: Ctrl+1..4 Launcher/Utils | Ctrl+5 Portapapeles | Ctrl+6 Snippets | Ctrl+7 Sync E2EE y QR"
exec "$ROOT_DIR/target/release/fluxcast" "$@"
