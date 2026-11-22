# FluxCast

Monorepo de desarrollo para Fedora KDE Plasma 6 (Wayland) y Android nativo. El [plan de implementación](implementation_plan.md) detalla las etapas del proyecto.

- **Etapa 0:** entorno de compilación en Fedora KDE, workspace de Rust, base Tauri v2, proyecto Android Gradle y flujos de GitHub Actions.
- **Etapa 1:** launcher nativo en Fedora KDE Plasma con búsqueda difusa (`nucleo-matcher`), indexador de apps `.desktop` + Flatpak, repositorios Git (Dolphin, VS Code, Konsole, Neovim), utilidades offline (`=`, `b64`/`unb64`, `hex`/`bin`/`dec`, `ts`, `uuid4`/`uuid7`, `jwt`), acciones rápidas de Plasma, atajo global nativo vía `org.freedesktop.portal.GlobalShortcuts` (`Alt+Space`) y respaldo instantáneo por UNIX socket (`fluxcast toggle`).
- **Etapa 2:** daemon de portapapeles Wayland sobre `ext-data-control-v1` (KWin) con fallback a `zwlr_data_control_manager_v1`, filtrado estricto de contraseñas (`x-kde-passwordManagerHint=secret`), modo pausa/incógnito (5 min, 15 min o manual), historial persistente en SQLite + FTS5 con clasificador automático (`JSON`, `URL`, `código`, `color HEX/RGB`, `JWT`, texto), fijado (`Pin`), pegado limpio (`Shift+Enter`) y bóveda de snippets con etiquetas, alias (`!dprune`, `!klog`), plantillas (`{{clipboard}}`, `{{date}}`, `{{uuid}}`) y ranking por frecuencia.

## Preparar Fedora KDE

Instala las dependencias nativas de compilación, empaquetado (`.rpm` / `AppImage`) y Podman Compose.

```sh
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel sqlite-devel wl-clipboard \
  pkgconf-pkg-config gcc-c++ rpm-build patchelf fuse fuse-libs podman-compose
```

Instala Rust estable con rustup y usa Node.js 22.12 o posterior. Desde la raíz del repositorio, prepara pnpm y los dos targets nativos de Android.

```sh
npm install --global pnpm@10.32.1
rustup target add aarch64-linux-android x86_64-linux-android
cargo install --locked --version 4.1.2 cargo-ndk
pnpm install --frozen-lockfile
```

Instala un JDK 17 (por ejemplo, Temurin) para Gradle. En Android Studio, abre el SDK Manager e instala Android SDK Command-line Tools. En Linux, si usas la ruta habitual del SDK, prepara los componentes del proyecto así.

```sh
export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
  "platforms;android-35" "build-tools;34.0.0" "ndk;27.2.12479018"
```

Configura `JAVA_HOME` con el directorio del JDK 17 para ejecutar Gradle (o instálalo bajo `~/.jdks/` para que Gradle lo detecte automáticamente con `gradle-daemon-jvm.properties`). Android Studio también permite seleccionarlo como Gradle JDK al abrir `apps/android`.

## Usar el launcher en Fedora KDE (Etapa 1)

Compila la aplicación de escritorio y ejecuta el binario `fluxcast`:

```sh
pnpm --filter @fluxcast/desktop tauri build --no-bundle
./target/release/fluxcast
```

También puedes abrirlo en modo desarrollo con `pnpm --filter @fluxcast/desktop tauri dev`.

- **Atajo global nativo:** al iniciar, FluxCast registra `toggle_launcher` (`Alt+Space`) en `xdg-desktop-portal-kde`. Puedes reasignarlo en *Preferencias del Sistema → Accesos rápidos*.
- **Alternado instantáneo por socket (<5 ms):** ejecuta `fluxcast toggle` (o `fluxcast` si ya está activo) para mostrar u ocultar la ventana desde scripts o atajos personalizados de KWin.
- **Atajos internos:** `↑`/`↓` navegar, `Enter` abrir/copiar/ejecutar, `Esc` ocultar, `Ctrl+1..4` cambiar pestaña, `Alt+1..4` elegir editor para proyectos Git, `Ctrl+R` reindexar apps y repositorios.
- **Directorios de proyectos Git:** por defecto escanea `~/Proyectos`, `~/Projects` y `~/Dev`; puedes personalizarlos con `FLUXCAST_PROJECT_DIRS="/ruta/a:/ruta/b"`.

## Verificar el proyecto

Ejecuta desde la raíz:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
pnpm --filter @fluxcast/desktop test
pnpm --filter @fluxcast/desktop tauri build --no-bundle
python3 scripts/test-launcher.py
./gradlew testDebugUnitTest
```

Para generar los instaladores en Fedora ejecuta `pnpm --filter @fluxcast/desktop tauri build --bundles rpm,appimage`. Para generar el APK de Android ejecuta `./gradlew assembleDebug` (salida en `apps/android/app/build/outputs/apk/debug/app-debug.apk`).

GitHub Actions comprueba formato, Clippy y pruebas de Rust, compila el escritorio Tauri y construye las dos bibliotecas nativas de Android junto con las pruebas y el APK de depuración.
