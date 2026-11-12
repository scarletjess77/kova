# How Kova works

## Layout

```
crates/kova-core     No I/O. Node model, link/subscription parsing, core capability table,
                     Mihomo YAML and Xray JSON generation.
crates/kova-engine   Storage, subscription download, core install/update, process supervision,
                     Mihomo controller client, Xray metrics, system proxy, routing data, the Engine.
crates/kova-service  The privileged Windows service that runs Mihomo for TUN mode.
src-tauri            The app shell: Tauri commands and events, tray menu, Android VPN service.
ui                   Svelte 5 + TypeScript interface.
```

## One profile, both cores

Mihomo runs in front: rules, groups, TUN, connection tracking. Nodes only Xray can run, such as VLESS over XHTTP, are bridged through a local Xray sidecar automatically. NaiveProxy servers, which neither core runs, go through [Shoes](https://github.com/cfal/shoes) the same way on Linux and macOS; Kova downloads it the first time a profile needs it (Shoes publishes no Windows build). A profile can also be pinned to Mihomo only or Xray only.

Clash/Mihomo YAML profiles are kept verbatim, so their groups and rules survive. Share links, base64 subscriptions and sing-box JSON are converted.

Cores ship with the app, at versions pinned in `scripts/android-cores.json` and tested with Kova, and can be updated from their official GitHub releases in Settings; every download is checked by SHA-256 before install. Cores run in a Windows Job Object so they can't outlive the app; on Linux and macOS a core left by a crashed Kova is ended at the next start. The core controller listens on localhost only and requires a per-install secret.

## TUN mode and the Kova service

TUN captures all traffic, including games and UDP, but creating the virtual adapter needs system rights. Instead of running Kova as administrator, Kova uses a small Windows service:

- **Installing:** the installer registers it (Kova is a per-machine install, so the admin prompt appears once). Without the installer, use **Settings → TUN mode → Install**. The same `kova.exe` acts as the service; it copies itself to `C:\Program Files\Kova Service\` and registers `KovaService` (automatic start).
- **What it does:** only starts and stops Mihomo. The rest of Kova (profiles, rules, statistics, the Xray bridge) runs as the user.
- **Safety:** it runs only its own Mihomo, kept in `C:\Program Files\Kova Service\cores`, which ordinary programs can't modify. It sanitises every config (local controller with a secret, no web UI, no extra control channels), and Mihomo keeps provider files inside its home folder. Mihomo's controller there has a secret only the service knows: Kova's requests go through a gate in the service that passes on only what Kova needs (reading, choosing a server, closing connections, the mode), so another program can't use Kova's secret to load a config of its own into the running Mihomo. Its named pipe refuses remote clients, and Kova sends nothing to a pipe that SYSTEM or an administrator didn't create, so another program can't pose as the service while it is stopped. The installer gives Kova's own folder the rights Program Files has, also when it is on another drive. It stops Mihomo as soon as the Kova app that started it exits.
- **Removing:** Settings → TUN mode → Remove, or uninstalling Kova.

Command line, from an elevated prompt: `kova.exe --service-install`, `kova.exe --service-uninstall`. For development, `kova.exe --service-console` runs the pipe server in the foreground (set `KOVA_SERVICE_HOME` and `KOVA_SERVICE_PIPE` to keep it separate from an installed service).

**Linux.** TUN needs the network-admin capability. Settings → TUN mode → **Give rights** asks for the password once (polkit) and installs a copy of Kova's Mihomo as `/usr/local/lib/kova/mihomo`, with `cap_net_admin`, `cap_net_bind_service` and `cap_net_raw` only. Only the user who gave the rights may run the copy (not their group, which some systems share between all accounts): file capabilities apply to whoever runs a file, so a copy others could run would give them those rights. TUN runs with that copy; Kova itself never runs as root. After a Mihomo update the copy is offered for granting again, and so is a copy from an earlier Kova, which others could run. Without polkit: `sudo install -D -m 0500 -o "$(id -u)" -g 0 <Mihomo> /usr/local/lib/kova/mihomo && sudo setcap cap_net_admin,cap_net_bind_service,cap_net_raw+ep /usr/local/lib/kova/mihomo`.

**macOS.** TUN still needs Kova started as administrator; a helper like the Windows service is planned.

## Routing data (GeoIP / GeoSite)

Rules such as `GEOSITE,category-ru` and `GEOIP,RU` read two files in the v2ray format, `geoip.dat` and `geosite.dat`. Both cores use the same copy. The source is chosen in Settings → Routing data:

| Source | Contents | Update |
|---|---|---|
| Standard | [MetaCubeX/meta-rules-dat](https://github.com/MetaCubeX/meta-rules-dat): every country and the v2fly site categories | weekly |
| Russia | [runetfreedom/russia-v2ray-rules-dat](https://github.com/runetfreedom/russia-v2ray-rules-dat): the standard set plus `ru-blocked`, `ru-available-only-inside`, `refilter`, `win-spy`, … | every 12 hours |
| Custom | any two direct links to `geoip.dat` and `geosite.dat` | weekly |

Standard and Russia downloads are checked against the SHA-256 digests GitHub publishes for each release; custom files are only checked to be readable geodata. Kova refuses to connect, and says why, if an enabled preset needs a category the files don't have.

**Where GitHub is blocked.** The installers carry the cores and the standard routing data. If the data still can't be downloaded, Kova connects without the rules that need it, downloads the data through the server, and then restarts the connection with every rule in place.

The Russia source enables the **Only blocked sites through the proxy** preset (Rules): sites and IP ranges in `ru-blocked` use the proxy, everything else goes direct.

**Where the files are** (`%APPDATA%\app.kova.client` on Windows, or `KOVA_DATA_DIR`):

- `cores\geoip.dat`, `cores\geosite.dat` and `cores\geodata.json` (source and date). Xray reads these directly.
- `runtime\main\mihomo\GeoIP.dat` and `GeoSite.dat`, linked from `cores` when they change, so the lists are on disk once: a hard link, or on Android, which allows apps none, a symbolic link (a copy only where neither works).
- `cores\geodata-cache\`: each source's last download, so switching back to it soon needs no download. A source that is not in use is removed once its copy is out of date (12 hours for Russia, a week for the others).
- With the Kova service: its own copy in `C:\Program Files\Kova Service\cores`, refreshed on a source switch.

Custom files can also be dropped into `cores`; they are used until the next scheduled update replaces them.

## Smart routing (beta)

Rules → **Smart routing**, off by default. Kova learns sites that fail on their usual path and routes them the other way: a site that is blocked when going direct goes through the proxy, and a site that refuses the proxy (one that blocks foreign IPs) goes direct. The presets cover known blocks; smart routing catches new ones, blocks of one ISP, and the extra domains a blocked site loads.

How it decides, in Rule mode with Mihomo in front:

1. **Suspects.** Mihomo reports every connection it could not open; Kova reads these warnings through the controller. It also flags a direct HTTPS connection that sent its greeting and got nothing back for 5 seconds, which is how blocking that drops packets looks. Only connections that reached the catch-all rule count: a rule chosen on purpose is never second-guessed.
2. **Check.** Mihomo gets two extra loopback ports that ignore the rules, one always direct and one always through the main group. Kova fetches the site both ways. It is learned only if the usual path fails twice, the other path works, and both paths can reach a known-good site.
3. **Apply.** Learned sites (the registrable domain, via the public suffix list) go into two rule lists that Mihomo fetches from Kova over loopback, right after the user's rules and presets. Mihomo reloads them without a restart; connections stuck on the failing path are closed so the page retries on the new route.

When a rule chosen on purpose (the user's, a preset's or the subscription's) sends a site somewhere it fails, Kova suggests a change on the Rules page instead of applying it.

**TUN mode** works the same way, also with Mihomo inside the Kova service. TUN also captures apps that connect to a bare IP address (Telegram, voice chats, games); such an address is learned toward the proxy when it can't be reached directly. Private, special and fake-IP ranges are never learned. Keep **Sniff domains** or **Fake-IP** on (both are by default) so Kova sees site names.

Learned sites are stored in `learned.json` in the data folder and expire after 30 days unless learned again. **Keep** turns one into a permanent rule. The first visit to a newly blocked site still fails once; throttling is not detected yet.

## Anti-DPI (beta)

Settings → Anti-DPI dials every server connection through the local Xray, which sends the start of each connection in a shape simpler filtering doesn't recognise: the TLS greeting in pieces of 100–200 bytes, 10–20 ms apart, and UDP connections (Hysteria2, TUIC) opened with 10–20 random bytes. It uses Xray's Finalmask (`fragment` and `noise`). With an Xray older than v26.3.23, which has neither, the same is done through the Direct outbound's `fragment` and `noises`, which newer Xray warns are going away.

A server whose link turns certificate checks off is asked for its certificate once before connecting, to pin it; with Anti-DPI on, that greeting is cut the same way.

## Android

- **The VPN.** Android lets an app capture traffic only through its VpnService. The Kotlin part (`src-tauri/gen/android/app/src/main/java/app/kova/client/`) asks for permission, opens the VPN interface and hands its descriptor to Mihomo, which uses it as its TUN device. Kova's own traffic is excluded from the VPN so it can reach the servers. Settings → VPN chooses which apps use it.
- **The cores ship inside the APK** as `libmihomo.so` and `libxray.so`, because Android only runs programs that come with the app. They are pinned in `scripts/android-cores.json` and updated together with the app.
- **TV.** The interface switches to a remote-friendly layout: the arrows move between controls row by row, Left and Right move between the menu and the page.
- **Updates** are downloaded inside the app, checked by SHA-256 and installed after Android asks to confirm. Every release is signed with the same key, so each installs over the last and keeps the profiles.

## For VPN providers

**"Add to Kova" links.** Kova opens `kova://import/<subscription address>` (the address appended as it is, like `happ://add/`), `kova://import?url=<encoded address>` and `kova://install-config?url=<encoded address>`. The link opens the Add profile dialog with the address filled in; the user presses Import. Share links (`vless://…`) work the same way.

The provider page, [scarletjess77.github.io/kova/providers.html](https://scarletjess77.github.io/kova/providers.html), has all of this ready to copy, in four languages. On Remnawave, add Kova in the dashboard's Subpage Builder with the link `kova://import/` before the subscription address; setups that still use `app-config.json` take this entry for each platform:

```json
{
  "id": "kova",
  "name": "Kova",
  "isFeatured": false,
  "urlScheme": "kova://import/",
  "installationStep": {
    "buttons": [
      { "buttonLink": "https://github.com/scarletjess77/kova/releases/latest", "buttonText": { "en": "Download", "ru": "Скачать" } }
    ],
    "description": { "en": "Download and install Kova", "ru": "Скачайте и установите Kova" }
  },
  "addSubscriptionStep": { "description": { "en": "Press the button to add the subscription, then Import", "ru": "Нажмите кнопку, чтобы добавить подписку, затем «Импортировать»" } },
  "connectAndUseStep": { "description": { "en": "Press the round button on the Dashboard", "ru": "Нажмите круглую кнопку на главной" } }
}
```

**Formats Kova reads:** a Clash/Mihomo config (kept as it is), sing-box JSON, Xray JSON (one config or a list of them, as panels send it for v2rayNG or Happ), plain or base64 share links, and WireGuard `.conf` files (a Cloudflare WARP profile, for example). From Xray JSON Kova takes each server with its transport and security, pinned certificate, ECH and mux; an outbound Kova doesn't read field by field, such as MASQUE, runs on Xray as the provider wrote it. The provider's fragment and outbound chains aren't used (Kova's Anti-DPI replaces fragment), and the profile lists what was left out. Mux comes in two kinds that don't mix: Xray's runs on Xray and sing-mux (Clash `smux`, sing-box `multiplex`) on Mihomo, so Automatic picks the core a server's mux needs. A profile can also send headers of its own with every update (Profiles → Extra headers) for providers that ask for one. A panel picks the format by User-Agent: Kova asks as `clash.meta`, and a profile's User-Agent can be changed in Profiles to get another format.

**Headers Kova reads:** `subscription-userinfo` (traffic and expiry), `profile-title`, `profile-update-interval`, `profile-web-page-url` or `support-url` (the Provider button) and `announce` (a message shown on the Dashboard; plain text or `base64:`).

**Headers Kova sends:** `User-Agent: clash.meta/1.19 mihomo Kova/<version>` (so panels return a full Mihomo config), `x-hwid` (a random ID made once per install, for device limits) and `x-device-os`.
