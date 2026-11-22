#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=================================================================="
echo "⚡ FluxCast · Verificación Integral Local (Etapas 0, 1, 2, 3 y 4)"
echo "=================================================================="

echo "==> [1/6] Formato y Linter de Rust (Workspace completo)..."
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings

echo "==> [2/6] Pruebas unitarias y de integración en Rust (Core, Crypto E2EE, Protocol HLC, UniFFI, Sync Server)..."
cargo test --locked --workspace

echo "==> [3/6] Compilando librerías nativas Android (.so con alineación 16 KB) y bindings Kotlin UniFFI..."
./scripts/build-android-rust.sh

echo "==> [4/6] Pruebas unitarias de UI Desktop (Vitest) y compilación release de Tauri para Fedora KDE..."
pnpm --filter @fluxcast/desktop test
pnpm --filter @fluxcast/desktop tauri build --no-bundle

echo "==> [5/6] Prueba E2E del Launcher en KWin Wayland virtual + D-Bus + Portal KDE + SQLite FTS5 + Snippets..."
python3 scripts/test-launcher.py

echo "==> [6/6] Pruebas unitarias JVM + UniFFI JNA y generación del APK de Android..."
./gradlew testDebugUnitTest assembleDebug

echo "=================================================================="
echo "✅ TODO EN VERDE · Binarios listos:"
echo "   • Fedora KDE Launcher: $ROOT_DIR/target/release/fluxcast"
echo "   • Sync Relay Server:   $ROOT_DIR/target/debug/fluxcast-sync-server"
echo "   • Android Debug APK:   $ROOT_DIR/apps/android/app/build/outputs/apk/debug/app-debug.apk"
echo "=================================================================="
