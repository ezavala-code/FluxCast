# FluxCast

Monorepo de desarrollo para Fedora KDE y Android. En la etapa 0 se puede abrir la ventana de escritorio, compilar la app Android y probar el workspace de Rust. El [plan de implementación](implementation_plan.md) describe las funciones de las siguientes etapas.

## Preparar Fedora KDE

Instala las dependencias nativas de compilación y Podman Compose.

```sh
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel sqlite-devel wl-clipboard \
  pkgconf-pkg-config gcc-c++ podman-compose
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

Configura `JAVA_HOME` con el directorio del JDK 17 para ejecutar Gradle. Android Studio también permite seleccionarlo como Gradle JDK al abrir `apps/android`.

## Verificar la etapa 0

Ejecuta desde la raíz. Las pruebas unitarias de Android corren en la JVM y comprueban la actividad de inicio declarada en el manifiesto fusionado.

```sh
cargo test --workspace
./gradlew testDebugUnitTest
```

Para abrir el escritorio en Fedora, ejecuta `pnpm --filter @fluxcast/desktop tauri dev`. Para generar el APK, ejecuta `./gradlew assembleDebug`. Gradle lo escribe en `apps/android/app/build/outputs/apk/debug/app-debug.apk`.

GitHub Actions comprueba formato, Clippy y pruebas de Rust, compila el escritorio Tauri y construye las dos bibliotecas nativas de Android junto con las pruebas y el APK de depuración.
