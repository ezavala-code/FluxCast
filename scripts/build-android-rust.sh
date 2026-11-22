#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  if [[ -n "${ANDROID_HOME:-}" && -d "$ANDROID_HOME/ndk/27.2.12479018" ]]; then
    export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
  elif [[ -d "$HOME/Android/Sdk/ndk/27.2.12479018" ]]; then
    export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/27.2.12479018"
  fi
fi

JNI_OUT="$ROOT_DIR/apps/android/app/src/main/jniLibs"
KOTLIN_OUT="$ROOT_DIR/apps/android/app/src/main/java"
mkdir -p "$JNI_OUT" "$KOTLIN_OUT"

# Alineación de páginas ELF a 16 KB requerida por Android 15+ (NDK r27+).
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"
export CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

echo "==> Compilando libfluxcast_uniffi.so para Android (arm64-v8a y x86_64) con alineación de 16 KB..."
cargo ndk -t arm64-v8a -t x86_64 -o "$JNI_OUT" build --release -p fluxcast-uniffi

echo "==> Compilando biblioteca host para inspección de metadatos UniFFI..."
cargo build -p fluxcast-uniffi

echo "==> Generando bindings Kotlin con uniffi-bindgen..."
rm -rf "$KOTLIN_OUT/uniffi"
cargo run -p fluxcast-uniffi --features cli --bin uniffi-bindgen -- \
  generate --language kotlin --no-format \
  --config "$ROOT_DIR/crates/fluxcast-uniffi/uniffi.toml" \
  --out-dir "$KOTLIN_OUT" \
  "$ROOT_DIR/target/debug/libfluxcast_uniffi.so"

echo "OK · Librerías .so en $JNI_OUT y bindings Kotlin en $KOTLIN_OUT/dev/fluxcast/core"
