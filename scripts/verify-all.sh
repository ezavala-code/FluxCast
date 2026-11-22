#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=================================================================="
echo "⚡ FluxCast · Verificación Integral Local (Etapas 0, 1, 2, 3, 4 y 5)"
echo "=================================================================="

echo "==> [1/7] Formato y Linter de Rust (Workspace completo)..."
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings

echo "==> [2/7] Pruebas unitarias y de integración en Rust (Core, Crypto E2EE, Protocol HLC, UniFFI, Sync Server)..."
cargo test --locked --workspace
cargo test --release -p fluxcast-core --test launcher search_latency -- --ignored --nocapture

echo "==> [3/7] Compilando librerías nativas Android (.so con alineación 16 KB) y bindings Kotlin UniFFI..."
./scripts/build-android-rust.sh

echo "==> [4/7] Pruebas unitarias de UI Desktop (Vitest) y empaquetado .rpm nativo para Fedora KDE..."
pnpm --filter @fluxcast/desktop test
pnpm --filter @fluxcast/desktop tauri build --bundles rpm

echo "==> [5/7] Prueba E2E del Launcher en KWin Wayland virtual + D-Bus + Portal KDE + SQLite FTS5 + Snippets..."
python3 scripts/test-launcher.py

echo "==> [6/7] Métricas reales de Etapa 5 (RAM RSS, latencia Delta Sync y prueba de install-fedora-kde.sh)..."
python3 scripts/measure-metrics.py

echo "==> [7/7] Pruebas unitarias JVM + UniFFI JNA y generación del APK de Android..."
./gradlew testDebugUnitTest assembleDebug

RPM_PKG="$(find "$ROOT_DIR/target/release/bundle/rpm" -name '*.rpm' | head -n 1)"
echo "=================================================================="
echo "✅ TODO EN VERDE (ETAPAS 0–5 COMPLETAS) · Artefactos generados:"
echo "   • Fedora KDE Launcher: $ROOT_DIR/target/release/fluxcast"
echo "   • Paquete Fedora RPM:  ${RPM_PKG:-target/release/bundle/rpm/}"
echo "   • Sync Relay Server:   $ROOT_DIR/target/debug/fluxcast-sync-server"
echo "   • Android Debug APK:   $ROOT_DIR/apps/android/app/build/outputs/apk/debug/app-debug.apk"
echo "=================================================================="
