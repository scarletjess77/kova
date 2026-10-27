<p align="center"><img src="docs/icon.png" width="72" alt=""></p>
<h1 align="center">Kova</h1>
<p align="center">A VPN client for the <b>Mihomo</b> and <b>Xray</b> cores, written in Rust.<br>Windows · Android (phones and TV) · Linux · macOS</p>
<p align="center">
  <a href="https://github.com/scarletjess77/kova/releases/latest"><img src="https://img.shields.io/github/v/release/scarletjess77/kova?label=download&color=222" alt="Download"></a>
  <a href="https://scarletjess77.github.io/kova/"><img src="https://img.shields.io/badge/website-kova-222" alt="Website"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/scarletjess77/kova?color=222" alt="License"></a>
</p>

![Kova on Windows and Android](.github/screenshots/hero.png)

## Features

- **Both cores in one app.** Mihomo runs in front; nodes only Xray can run (such as VLESS over XHTTP) are bridged through Xray automatically.
- **Any subscription.** Clash/Mihomo YAML kept as is, share links (`vless`, `vmess`, `trojan`, `ss`, `hysteria2`, `tuic`, `wireguard`, `anytls`, `socks`), base64 and sing-box JSON. Import from the clipboard, a file, a QR code, or another device on the same network.
- **Routing.** Presets (Russia, China, Iran direct; ad blocking; only blocked sites through the proxy), rules for sites and apps, and [smart routing](docs/how-it-works.md#smart-routing-beta), which learns sites that fail on their usual path.
- **Works where GitHub is blocked.** Cores and routing data ship with the app; missing data is fetched through the server once connected.
- **Checks.** "What's wrong?" diagnostics, a site check (direct and through Kova), a privacy check (exit IP, encrypted DNS, IPv6 leaks) and a speed test.
- **TUN mode** on Windows without running as administrator, through a small [service](docs/how-it-works.md#tun-mode-and-the-kova-service). On Android: a per-app VPN, a quick-settings tile, a widget and always-on support.
- **Safe by default.** Downloads are checked by SHA-256, updates are signed, the core controller listens on localhost only with a secret, and the system proxy is restored after a crash.
- **English and Russian**, and 11 themes: dark, light, OLED black, dim, warm paper, high contrast, Nord, Catppuccin, Dracula, Gruvbox and Solarized.

<p>
  <img src=".github/screenshots/proxies.png" width="49%" alt="Proxies">
  <img src=".github/screenshots/rules.png" width="49%" alt="Rules">
</p>

## Download

The latest build is on the [Releases](https://github.com/scarletjess77/kova/releases/latest) page:

| System | File |
|---|---|
| Windows 10/11 | `_x64-setup.exe` |
| Android 7+, phones and TV | `_android-arm64.apk` (most devices); `_android-armv7.apk` (older and 32-bit devices, many TVs) |
| Linux | `.deb` (includes the cores) or `.AppImage` (smaller; downloads the cores on first start) |
| macOS | `_aarch64.dmg` (Apple silicon); `_x64.dmg` (Intel) |

Kova updates itself (Settings → Updates). Windows and Android are the most tested; Linux and macOS builds are newer. The Windows installer is not code-signed yet, so SmartScreen may warn.

## Privacy

No accounts, ads or telemetry. Kova contacts other services only for these features:

- **Exit point** on the dashboard: `ipwho.is` (or `ipinfo.io`).
- **Privacy check:** `ipify.org`. **Speed test:** `speed.cloudflare.com`.
- **Site check:** the sites in its list, only when it is run.
- **Updates:** `github.com`, for Kova, the cores and the routing data.
- **Subscription updates** carry a random device ID made once per install (not a hardware ID); providers with device limits count devices by it.

## Documentation

- [How it works](docs/how-it-works.md): the two cores, TUN mode and the Kova service, routing data, smart routing, Android.
- [Building and testing](docs/building.md): desktop and Android builds, tests, releases.
- [Security policy](SECURITY.md): how to report a vulnerability.

## Support

Kova is free. Development can be supported with TON:

```
UQCfiJW3GM9sEfYS2fsk-q9WNbKXXn3m0KU17Dx5XTCs3fv0
```

The same address, with a QR code, is under **Donate** in the app.

## License

Copyright © 2026 scarletjess77. [GPL-3.0-or-later](LICENSE).
