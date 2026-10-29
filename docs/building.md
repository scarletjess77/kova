# Building and testing

## Desktop

Requirements: Rust (stable), Node.js 20+, and on Windows the MSVC C++ build tools.

```bash
npm install
npm run tauri dev        # run with hot reload
npm run tauri build      # release build and installer
```

Set `KOVA_DATA_DIR` to keep development data apart from a real profile (`dev-data/` is ignored by git). On first launch Kova downloads any missing core.

## Android

Requirements: Android SDK with platform 36 and NDK r28, JDK 17 or 21, Go, and the Rust targets `aarch64-linux-android` (and `armv7-linux-androideabi` for 32-bit devices). With `ANDROID_HOME`, `NDK_HOME` and `JAVA_HOME` set:

```bash
npm run android:build                      # release APK, arm64 (unsigned unless KOVA_ANDROID_KEYSTORE is set)
npm run android:cores && npx tauri android build --apk --target aarch64 --debug   # debug APK, installable as is
node scripts/android-cores.mjs armeabi-v7a && npx tauri android build --apk --target armv7   # 32-bit
```

`scripts/android-cores.mjs` gets the cores pinned in `android-cores.json`: Xray's official Android build (checked by SHA-256) for arm64, and Mihomo built from its official source with Go (and Xray too for 32-bit, which has no official Android build). The APK is in `src-tauri/gen/android/app/build/outputs/apk/`.

## Tests

```bash
cargo test                                                              # unit tests
cargo test -p kova-engine --test e2e -- --ignored --nocapture           # real cores, local server
cargo test -p kova-engine --test geodata -- --ignored --nocapture       # downloads routing data
cargo test -p kova-service --test service_e2e -- --ignored --nocapture  # service path (run e2e first)
npm run check                                                           # UI types and translations
```

The end-to-end test downloads the real cores, starts a local Xray server with VLESS over WebSocket and XHTTP, imports both as share links, tests latency, connects, and fetches a page through each node, including the bridged XHTTP one. It also checks switching away from a dead node and connecting without routing data. None of the tests touch the system proxy or enable TUN.

`npm run check` also checks that every interface text has a Russian translation (`ui/src/lib/lang/ru.ts`; the English text is the key).

## Releases

The cores every build ships, desktop installers included, are the versions pinned in `scripts/android-cores.json`; the end-to-end tests run against the same. To move to a new Mihomo or Xray, change the pin (the **Core versions** workflow opens an issue when one is out) and run the end-to-end tests.

`node scripts/bump-version.mjs <version>` sets the version everywhere. A `v<version>` tag starts the **Release** workflow, which builds and signs every installer and APK into a draft release. Installed copies see the update once the draft is published. The **CI** workflow runs the tests on Windows, Linux and macOS for every push.
