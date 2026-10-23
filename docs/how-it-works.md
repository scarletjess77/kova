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

Mihomo runs in front: rules, groups, TUN, connection tracking. Nodes only Xray can run, such as VLESS over XHTTP, are bridged through a local Xray sidecar automatically. A profile can also be pinned to Mihomo only or Xray only.

Clash/Mihomo YAML profiles are kept verbatim, so their groups and rules survive. Share links, base64 subscriptions and sing-box JSON are converted.

Cores ship with the app and are updated from their official GitHub releases, checked by SHA-256 before install. Cores run in a Windows Job Object so they can't outlive the app. The core controller listens on localhost only and requires a per-install secret.

## TUN mode and the Kova service

TUN captures all traffic, including games and UDP, but creating the virtual adapter needs system rights. Instead of running Kova as administrator, Kova uses a small Windows service:

- **Installing:** the installer registers it (Kova is a per-machine install, so the admin prompt appears once). Without the installer, use **Settings → TUN mode → Install**. The same `kova.exe` acts as the service; it copies itself to `C:\Program Files\Kova Service\` and registers `KovaService` (automatic start).
- **What it does:** only starts and stops Mihomo. The rest of Kova (profiles, rules, statistics, the Xray bridge) runs as the user.
- **Safety:** it runs only its own Mihomo, kept in `C:\Program Files\Kova Service\cores`, which ordinary programs can't modify. It sanitises every config (local controller with a secret, no web UI, no extra control channels), and Mihomo keeps provider files inside its home folder. Its named pipe refuses remote clients and can't be squatted. It stops Mihomo as soon as the Kova app that started it exits.
- **Removing:** Settings → TUN mode → Remove, or uninstalling Kova.

Command line, from an elevated prompt: `kova.exe --service-install`, `kova.exe --service-uninstall`. For development, `kova.exe --service-console` runs the pipe server in the foreground (set `KOVA_SERVICE_HOME` and `KOVA_SERVICE_PIPE` to keep it separate from an installed service).

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
- `runtime\main\mihomo\GeoIP.dat` and `GeoSite.dat`, copied from `cores` when they change.
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

## Android

- **The VPN.** Android lets an app capture traffic only through its VpnService. The Kotlin part (`src-tauri/gen/android/app/src/main/java/app/kova/client/`) asks for permission, opens the VPN interface and hands its descriptor to Mihomo, which uses it as its TUN device. Kova's own traffic is excluded from the VPN so it can reach the servers. Settings → VPN chooses which apps use it.
- **The cores ship inside the APK** as `libmihomo.so` and `libxray.so`, because Android only runs programs that come with the app. They are pinned in `scripts/android-cores.json` and updated together with the app.
- **TV.** The interface switches to a remote-friendly layout: the arrows move between controls row by row, Left and Right move between the menu and the page.
- **Updates** are downloaded inside the app, checked by SHA-256 and installed after Android asks to confirm. Every release is signed with the same key, so each installs over the last and keeps the profiles.
