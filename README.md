# Kova

A desktop VPN client for the **Mihomo** and **Xray** cores, written in Rust. It aims to replace v2rayN, Clash Verge Rev and similar clients with one app that runs either core, or both at once.

Windows is the first target; Linux and macOS share the same code, and Android is planned.

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

## Tests

```bash
cargo test                                                              # unit tests
cargo test -p kova-engine --test e2e -- --ignored --nocapture           # real cores, local server
cargo test -p kova-engine --test geodata -- --ignored --nocapture       # downloads routing data
cargo test -p kova-service --test service_e2e -- --ignored --nocapture  # service path (run e2e first)
```

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

Not done yet:

- TUN without admin rights on Linux and macOS (Windows has the Kova service).
- Linux and macOS: system-proxy code is written but untested.
- Interface languages other than English (Russian first), QR code import, global shortcuts, Xray JSON import.
- Android (shared `kova-core`, VpnService, cores built as libraries).
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

The **Release** workflow builds and signs the installer and creates a draft release. Publish the draft on GitHub; installed copies see the update from then on. The **CI** workflow runs the tests on Windows, Linux and macOS for every push.

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
