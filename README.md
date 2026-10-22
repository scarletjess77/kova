# Kova

A desktop VPN client for the **Mihomo** and **Xray** cores, written in Rust. It aims to replace v2rayN, Clash Verge Rev and similar clients with one app that runs either core, or both at once.

Windows is the first target; Linux and macOS share the same code, and an early Android app (arm64) comes with every release.

By **scarletjess77** · [github.com/scarletjess77/kova](https://github.com/scarletjess77/kova)

## What makes it different

- **One profile, both cores.** Mihomo runs in front (rules, groups, TUN, connection tracking). Nodes only Xray can run, such as VLESS over XHTTP, are bridged through a local Xray sidecar automatically. You can also pin a profile to Mihomo only or Xray only.
- **Every common format in, nothing lost.** Clash/Mihomo YAML profiles are kept verbatim (their groups and rules survive); share links (`vless`, `vmess`, `trojan`, `ss`, `hysteria2`/`hy2`, `tuic`, `wireguard`, `anytls`, `socks`), base64 subscriptions and sing-box JSON are converted.
- **Safe by default.** Cores are downloaded from the official GitHub releases and their SHA-256 is checked before install. The system proxy is restored on disconnect, on quit, and on the next launch after a crash. Cores run in a Windows Job Object so they can't outlive the app. The core controller listens on localhost only and requires a per-install secret.

## Layout

```
crates/kova-core     No I/O. Node model, link/subscription parsing, core capability table,
                     Mihomo YAML and Xray JSON generation. Reusable on Android.
crates/kova-engine   Storage, subscription download, core install/update, process supervision,
                     Mihomo controller client, Xray metrics, system proxy, routing data, the Engine.
crates/kova-service  The privileged Windows service that runs Mihomo for TUN mode.
src-tauri            Desktop shell: Tauri commands and events, tray menu, window lifecycle.
ui                   Svelte 5 + TypeScript frontend.
design               The approved UI prototype and logo source.
```

## Build

Requirements: Rust (stable), Node.js 20+, and on Windows the MSVC C++ build tools.

```bash
npm install
npm run tauri dev        # run with hot reload
npm run tauri build      # release build and installer
```

Set `KOVA_DATA_DIR` to keep development data away from your real profile (`dev-data/` is ignored by git). On first launch Kova downloads any missing core.

### Android (early)

Each release has an APK (`Kova_<version>_android-arm64.apk`) next to the Windows installer. The same app runs on Android (arm64, Android 7+) through Tauri: the same interface, laid out for a phone, and the same engine. What differs:

- **The VPN.** Android lets an app capture traffic only through its VpnService. The Kotlin part (`src-tauri/gen/android/app/src/main/java/app/kova/client/`) asks for permission, opens the VPN interface and hands its descriptor to Mihomo, which uses it as its TUN device. Kova's own traffic, the cores' included, is excluded from the VPN so it can reach the servers. There is no system proxy, TUN switch or Kova service on Android; the VPN covers all apps.
- **The cores ship inside the APK** as `libmihomo.so` and `libxray.so`, because Android only runs programs that come with the app. `npm run android:cores` downloads Xray's Android build from its official release and checks its SHA-256, and builds Mihomo from its official source with Go (the `cmfa` build Clash Meta for Android uses, which doesn't need root); they are updated together with the app.
- **HTTPS** uses the certificates the device trusts.
- **Updates.** Settings → Updates finds a newer APK in the releases (betas too with **Get beta versions**) and opens it in the browser; Android asks to confirm the install. Releases are all signed with the same key, so each installs over the last and keeps your profiles.

Requirements: Android SDK with platform 36 and NDK r28, JDK 17 or 21, Go, and the Rust target `aarch64-linux-android`. With `ANDROID_HOME`, `NDK_HOME` and `JAVA_HOME` set:

```bash
npm run android:build                      # release APK (unsigned unless KOVA_ANDROID_KEYSTORE is set)
npm run android:cores && npx tauri android build --apk --target aarch64 --debug   # debug APK, installable as is
```

The APK is in `src-tauri/gen/android/app/build/outputs/apk/`.

## Tests

```bash
cargo test                                                              # unit tests
cargo test -p kova-engine --test e2e -- --ignored --nocapture           # real cores, local server
cargo test -p kova-engine --test geodata -- --ignored --nocapture       # downloads routing data
cargo test -p kova-service --test service_e2e -- --ignored --nocapture  # service path (run e2e first)
```

`npm run check` type-checks the UI and checks that every interface text has a Russian translation (`ui/src/lib/lang/ru.ts`; the English text is the key).

The end-to-end test downloads the real cores, starts a local Xray server with VLESS over WebSocket and XHTTP, imports both as share links, tests latency while disconnected, connects, and fetches a page through each node, including the bridged XHTTP one. The service test runs the service's pipe server in-process and sends traffic through Mihomo inside it. None of the tests touch the system proxy or enable TUN.

## TUN mode and the Kova service

TUN captures all traffic, including games and UDP, but creating the virtual adapter needs system rights. Instead of running Kova as administrator, Kova uses a small Windows service:

- **Installing:** the installer registers it (Kova is a per-machine install, so you approve the admin prompt once). Without the installer, use **Settings → TUN mode → Install**, which shows the admin prompt once. The same `kova.exe` acts as the service; it copies itself to `C:\Program Files\Kova Service\` and registers `KovaService` (automatic start).
- **What it does:** only starts and stops Mihomo. The rest of Kova (profiles, rules, statistics, the Xray bridge) runs as you.
- **Safety:** it runs only its own Mihomo, downloaded and SHA-256-verified into `C:\Program Files\Kova Service\cores`, which ordinary programs can't modify. It sanitises every config (local controller with a secret, no web UI, no extra control channels), and Mihomo keeps provider files inside its home folder. Its named pipe refuses remote clients and can't be squatted. It stops Mihomo as soon as the Kova app that started it exits.
- **Removing:** Settings → TUN mode → Remove, or uninstalling Kova.

Command line, from an elevated prompt: `kova.exe --service-install`, `kova.exe --service-uninstall`. For development, `kova.exe --service-console` runs the pipe server in the foreground (set `KOVA_SERVICE_HOME` and `KOVA_SERVICE_PIPE` to keep it separate from an installed service).

## Routing data (GeoIP / GeoSite)

Rules such as `GEOSITE,category-ru` and `GEOIP,RU` read two files in the v2ray format, `geoip.dat` and `geosite.dat`. Both cores use the same copy.

**Changing the source:** Settings → Routing data:

| Source | Contents | Update |
|---|---|---|
| Standard | [MetaCubeX/meta-rules-dat](https://github.com/MetaCubeX/meta-rules-dat): every country and the v2fly site categories | weekly |
| Russia | [runetfreedom/russia-v2ray-rules-dat](https://github.com/runetfreedom/russia-v2ray-rules-dat): the standard set plus `ru-blocked`, `ru-available-only-inside`, `refilter`, `win-spy`, … | every 12 hours |
| Custom | any two direct links to `geoip.dat` and `geosite.dat` | weekly |

Standard and Russia downloads are checked against the SHA-256 digests GitHub publishes for each release; custom files are only checked to be readable geodata. Choosing a source downloads it right away; **Update now** fetches it again. Kova refuses to connect, and says why, if an enabled preset needs a category the files don't have.

The Russia source enables the **Only blocked sites through the proxy** preset (Rules): sites and IP ranges in `ru-blocked` use the proxy, everything else goes direct.

To use runetfreedom as a custom source instead (for example a mirror), enter:

```
https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geoip.dat
https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geosite.dat
```

**Where the files are** (`%APPDATA%\app.kova.client` on Windows, or `KOVA_DATA_DIR`):

- `cores\geoip.dat`, `cores\geosite.dat` and `cores\geodata.json` (source and date). Xray reads these directly.
- `runtime\main\mihomo\GeoIP.dat` and `GeoSite.dat`, copied from `cores` when they change. Mihomo also refreshes these itself every 24 hours from the same source.
- With the Kova service: its own verified copy in `C:\Program Files\Kova Service\cores`, refreshed when you switch source.

You can also drop your own files into `cores`. They are used until the next scheduled update replaces them; to keep them, pick a source whose files you control.

## Smart routing (beta)

Rules → **Smart routing**, off by default. Kova learns sites that fail on their usual path and routes them the other way: a site that is blocked when going direct goes through the proxy, and a site that refuses the proxy (one that blocks foreign IPs) goes direct. It complements the routing presets: the lists cover known blocks, smart routing catches new ones, blocks of one ISP, and the extra domains a blocked site loads.

How it decides, in Rule mode with Mihomo in front:

1. **Suspects.** Mihomo reports every connection it could not open; Kova reads these warnings through the controller, whatever the log level. It also flags a direct HTTPS connection that sent its greeting and got nothing back for 5 seconds, which is how blocking that drops packets looks. Only connections that reached the catch-all rule count: a rule chosen on purpose, the user's or a preset's, is never second-guessed.
2. **Check.** Mihomo gets two extra loopback ports that ignore the rules, one always direct and one always through the main group. Kova fetches the site both ways. It is learned only if the usual path fails twice, the other path works, and both paths can reach a known-good site (otherwise the internet or the proxy is simply down).
3. **Apply.** Learned sites (the registrable domain, via the public suffix list) go into two rule lists that Mihomo fetches from Kova over loopback, right after your rules and presets. Mihomo reloads them on request, so nothing restarts; connections stuck on the failing path are closed so the page retries on the new route.

**TUN mode.** Everything above works the same with TUN, also when Mihomo runs inside the Kova service: the checks and the lists use loopback, which TUN leaves alone. TUN also captures apps that connect to a bare IP address without a name (Telegram, voice chats, games). Such an address is learned when Mihomo could not connect to it directly and it is reachable through the proxy; the check opens a plain connection through the probe ports' SOCKS5 side, so it works whatever protocol the app speaks. Addresses are only learned toward the proxy, and private, special and fake-IP ranges never. Kova needs site names to learn sites, so keep **Sniff domains** or **Fake-IP** on (both are on by default).

Learned sites are stored in `learned.json` in the data folder and expire after 30 days unless learned again. The list on the Rules page shows why each site was learned; **Keep** turns one into a rule of your own. The first visit to a newly blocked site still fails once; throttling (as opposed to blocking) is not detected yet.

## Status

Working and tested on Windows 11:

- Import from links, clipboard text or files; subscription updates with usage and expiry headers; auto-update.
- Connect and disconnect, Rule/Global/Direct modes, system proxy, hybrid Mihomo + Xray.
- Proxy groups, node selection (remembered per profile), latency tests with or without a connection.
- Live traffic, connections (with per-connection close), core logs, memory.
- Routing presets (Russia, China, Iran direct; ad blocking; only blocked sites through the proxy) and custom rules, also applied in front of Clash profiles.
- Routing data from MetaCubeX, runetfreedom or custom URLs, verified and kept up to date.
- The Kova service for TUN mode: tested end to end in-process; not yet tested as an installed Windows service with TUN switched on.
- Config editor that checks a profile with Mihomo before saving.
- Core install and update from Settings; tray menu; close to tray; single instance; launch at sign-in.
- English and Russian interface, following the Windows display language unless chosen in Settings.

Not done yet:

- TUN without admin rights on Linux and macOS (Windows has the Kova service).
- Linux and macOS: system-proxy code is written but untested.
- Error messages from the engine are still English only; QR code import, global shortcuts, Xray JSON import.
- Android: tested on one phone so far; no per-app VPN choice yet.
- Code-signed installers (Windows SmartScreen warns until the installer is signed with a certificate).

## Releases and in-app updates

Kova checks GitHub for a newer release on launch (Settings → Updates; can be turned off) and installs it in place. Updates are signed: Kova only installs packages signed with the project's updater key.

Each release:

```bash
node scripts/bump-version.mjs 0.2.0
git commit -am "Release 0.2.0"
git tag v0.2.0
git push origin HEAD v0.2.0
```

The **Release** workflow builds and signs the Windows installer and the Android APK and creates a draft release. Publish the draft on GitHub; installed copies see the update from then on. The **CI** workflow runs the tests on Windows, Linux and macOS for every push.

## Privacy note

The dashboard's "Exit point" card asks `ipwho.is` (falling back to `ipinfo.io`) for your public IP, through the proxy when connected. The update check asks `github.com` for the latest release. The site check contacts the sites in its list only when you run it.

## Support

Kova is free, with no ads or tracking. If it helps you, you can support its development with TON:

```
UQCfiJW3GM9sEfYS2fsk-q9WNbKXXn3m0KU17Dx5XTCs3fv0
```

The app shows the same address, with a QR code, under **Donate** in the title bar and in **Settings → Support Kova**.

## License

Copyright © 2026 scarletjess77. Licensed under the GNU General Public License v3.0 or later.
