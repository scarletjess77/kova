# Kova FAQ

Short answers about Kova. For every option see [reference.md](reference.md). For how the parts work see [how-it-works.md](how-it-works.md). Names in bold are the exact texts in the app.

## Getting started

### What is Kova?
A free VPN client for Windows, macOS, Linux and Android (phones and TV). It runs the Mihomo and Xray cores for you. You add servers from your provider and press the power button. Kova does not sell servers.

### Is Kova free? Does it collect data?
It is free (GPL-3.0-or-later). It has no accounts, ads or telemetry. It contacts only the addresses listed under **Settings → What Kova connects to**: GitHub for updates, your subscription address, and a few services for features you run yourself (exit address, privacy check, speed test, Site check). Subscription updates carry a random device ID made once per install, not a hardware ID.

### How do I start?
Open **Profiles**, press **Add profile**, paste your subscription link or share links, press **Import**. Then press the power button on the **Dashboard**. On a first start the Dashboard asks **Where are you?** so local sites can go direct.

### How do I import from another app?
Copy the subscription link or the share links from that app and paste them into **Add profile → Link** (or use **Clipboard**). Kova reads Clash/Mihomo YAML, sing-box JSON, Xray JSON, WireGuard `.conf`, base64 lists and `vless`, `vmess`, `trojan`, `ss`, `hysteria2`, `tuic`, `wireguard`, `anytls`, `socks` and `naive+https` links. From a phone, use **QR code** or **Another device**.

### GitHub is blocked where I live. Can I still use Kova?
Yes. The installers carry the cores and the standard routing data. If the routing data cannot be downloaded, Kova connects without the rules that need it, downloads the data through the server, then restarts the connection with every rule. A profile can also be fetched through the proxy (**Update through the proxy**).

### Which ports does Kova use?
The proxy port (HTTP and SOCKS5) is **7890** by default; change it in **Settings → Network** (1024 to 65534). Xray profiles also use the port above it, and with **Allow connections from LAN** and Xray the two ports above that. Kova opens other local ports on localhost for its own use (the core controller and helpers); these are picked free at each start.

## Connecting

### What is the difference between System proxy, TUN and the kill switch?
- **System proxy** sets the system's proxy to Kova's port. Browsers and most apps follow it. Games and programs that ignore it do not.
- **TUN mode** adds a virtual network adapter that captures every app, including games and UDP.
- **Kill switch** (beta, desktop) is a safety net: if the connection drops by itself, apps that use the system proxy go offline instead of going out directly, and Kova reconnects. It does not hold programs that ignore the system proxy.

### The Dashboard says "Proxy only". What does it mean?
Kova runs but neither the system proxy nor TUN is on, so apps are not sent through it. Turn on **System proxy** or **TUN mode**, or point an app at the proxy port by hand.

### Why does a server show no latency?
It has not been tested yet (press **Test latency** in Proxies or **Test speed** on the Dashboard), the test is running, or it did not answer (shown with a "No answer" hint). Tests go through each server to the **Latency test address** in **Settings → Servers**; if that address is blocked, change it.

### Why does "Fastest server" not switch at once?
It switches only when another server is faster by more than a tolerance, 50 ms by default, to avoid constant moving. Change it with **Switch to a faster server** in **Settings → Network** (the options read **if faster by 10 ms** up to **if faster by 200 ms**, or **Never**; new in 0.3.0). Press **Test latency** to refresh the numbers it uses. A group of this type chooses by itself, so you cannot pin a server in it; use a select group for that.

### What happens if my server stops working?
With **Switch away from a dead node** (on by default) Kova moves to the fastest working server and tells you.

### How do I choose a server?
On the **Dashboard**, **Quick switch**, or on **Proxies** click a server in a select group. Star servers you use often (new in 0.3.0) and press **Favorites** to see only those.

## Subscriptions and profiles

### Does Kova allow http:// subscriptions?
Yes, both `http://` and `https://` addresses are accepted for the subscription itself. Use https where you can, because an http link travels unencrypted. Separately, if a Clash profile points one of its server or rule providers at this computer (localhost, 127.0.0.1 and similar), Kova skips that provider, unless it is on the same host as the subscription.

### Why is my subscription refused or empty?
Kova shows the reason. Common ones: "The server refused access (401 or 403). The subscription may have expired"; "Nothing found at that address (404)"; "The server took too long to answer"; "The server sent something that is not a subscription"; "No servers found in it". A subscription larger than 20 MB is refused. Providers choose the format by User-Agent; if you get the wrong format, try another **User-Agent** in the profile (for example `v2rayNG/1.10.0`). A provider may also need **Extra headers**.

### How often do subscriptions update?
The profile's **Auto-update** setting: off, or every 1 hour up to every 3 days. New subscriptions use the provider's interval, or 12 hours. **Update now** and **Update all** run it by hand.

### Can I use several profiles at once?
Yes: **Profiles → All profiles → Use all profiles together** merges their servers into one list. Tick which profiles to include.

### Can I edit a profile?
**Edit config** edits the raw config, checked with Mihomo before saving. For a subscription the next update replaces your edits. Ports, controller and TUN are always managed by Kova.

### Why can one profile run on both Mihomo and Xray?
Mihomo runs in front and handles rules, groups and TUN. Servers only Xray can run (VLESS over XHTTP, Xray's mux, MASQUE) are bridged through a local Xray automatically. **Run on core** can pin a profile to **Mihomo only** or **Xray only**; **Automatic** is the default. Xray only has no live Connections list and no TUN.

### What are Mihomo, Xray and Shoes?
Mihomo and Xray are the open source proxy cores Kova drives. Shoes runs NaiveProxy servers, which neither core supports, on Linux and macOS; Kova installs it when a profile needs it.

### Does Kova support XHTTP and Reality?
Yes. VLESS over XHTTP is run through Xray, and Kova's server details show Reality public key and short ID fields. Kova picks the core for you.

## Routing and rules

### How do I route only one app?
**Rules → Your rules → Route an app...** (desktop), choose the program and press **Proxy**, **Direct** or **Block**, then **Apply**. With the system proxy only programs that use it are affected; **TUN mode** covers all programs. You can also add an **App (process name)** rule, such as `steam.exe`.

### How do I block or route torrents?
**Rules → Torrent clients** (desktop; new in 0.3.0): **Through the proxy**, **Direct** or **Block**. It matches known programs by name (qBittorrent, uTorrent, Transmission, Deluge and others) and only works for traffic that goes through Kova. Many providers forbid torrents; **Through the proxy** can get your account blocked.

### Why does my rule not work?
Rules are checked from the top; the first match wins, and your rules come before presets. Check that **Apply** was pressed, that the mode is **Rule** (**Global** and **Direct** skip rules), and that for an imported Clash profile **Apply to imported Clash profiles** is on. A rule by app only affects traffic that reaches Kova.

### How do I send AI services through another country?
In **Rules → Your rules**, use **ChatGPT, Claude, Gemini and other AI services through** and pick a country.

### What is smart routing?
A beta feature (**Rules → Smart routing**, off by default) that learns sites that fail on their usual path and sends them the way that works. Learned sites expire after 30 days unless you press **Keep**. See [how-it-works.md](how-it-works.md#smart-routing-beta).

## TUN and the Kova service

### Why does the Kova service exist, and why does it start with Windows?
TUN needs system rights. Instead of running the whole app as administrator, Kova installs a small Windows service that only starts and stops Mihomo. It is set to start automatically so TUN works whenever you connect. You can remove it in **Settings → TUN mode → Remove**.

### What does "Repair or update" do?
It re-installs the service. It appears when the service is not running or was installed by another Kova build. Windows asks for administrator approval once.

### How do I use TUN on Linux and macOS?
Linux: **Settings → TUN mode → Give rights** (asks for your password once). macOS: TUN needs Kova started with `sudo` for now; the system proxy works without it.

## Privacy and security

### Is Kova signed? How do I verify it?
Updates are signed, and Kova installs only updates signed with its key. Cores are checked by SHA-256 before installing. The Windows installer is not code-signed yet, so SmartScreen may warn. Downloads come from the repository's Releases page. Whether you can verify a downloaded installer by hand is not confirmed here.

### Where are my profiles and logs stored?
In the data folder: `%APPDATA%\app.kova.client` on Windows (open it with **Settings → App → Data folder**). Set the environment variable `KOVA_DATA_DIR` to use another folder. Subscriptions are stored encrypted on Windows and readable only by you on Linux and macOS. Log files are in the `logs` folder: `kova.log`, the previous one `kova.log.1`, and `crash.log`. The Windows service keeps its log only in memory. Core output on the **Logs** page shows only the current run.

### What does the report from "What's wrong?" contain?
**Copy report** (also on the **Logs** page) copies text with the Kova version and system, connection status and any error, main settings (mode, system proxy, TUN, DNS and routing switches, preset names, number of own rules), the active profile's format, node count and core, the check results, the last crash if there was one, and the last 200 log lines. Before copying, Kova replaces links, UUIDs, long tokens, Kova's own secrets and device ID, the names and addresses of your servers, and public IP addresses with placeholders. Read it once before posting.

### Where do I report a bug?
Open an issue on the repository's Issues page and paste the report text from **What's wrong? → Copy report**, with your Kova version and steps. Never post subscription links, server addresses or keys. For a security problem use the private reporting described in [SECURITY.md](../SECURITY.md).

### Is the kill switch enough to avoid leaks?
It holds only the system proxy. Use **TUN mode** to capture all apps, and check **Site check → Privacy check** for DNS and IPv6 leaks. On Android turn on Always-on VPN and **Block connections without VPN**.

## Troubleshooting

### Connected, but nothing opens. What do I check?
Press **What's wrong?** on the Dashboard; it names the failing step and offers a fix. Also try another server, check **System proxy** or **TUN mode** is on, and test the site in **Site check**.

### "Port is already used".
Another program uses Kova's port. Close it or choose another **Proxy port** in **Settings → Network**.

### Anti-DPI: how does it work and when does it not help?
**Anti-DPI** (beta, needs Xray) sends the start of each connection to the server in small pieces, and starts UDP connections (Hysteria2, TUIC) with random bytes, so simpler blocking equipment cannot recognise them. It does not help against whitelists, or against blocking of the server's address.

### Can I run two copies of Kova?
Not on desktop: starting Kova again brings up the window of the one already running. `KOVA_DATA_DIR` only changes where one copy keeps its data (used in development). If you want a second proxy at the same time, a different **Proxy port** is needed, but running two copies side by side is not confirmed.

## Android

### How do I choose which apps use the VPN?
**Settings → VPN → Apps through the VPN**: **All apps**, **Only chosen** or **All but chosen**. Saving reconnects.

### How do I use a different server on mobile data?
**Settings → VPN → Server on mobile data**. Kova switches to it on mobile data and back on Wi-Fi.

### Kova stops in the background.
Allow it under **Keep running in the background** (**Allow**). Some makers also need Autostart turned on; Settings shows a button on those phones. Android may otherwise stop Kova to save battery.

### How do I connect from the quick settings?
Swipe down twice, tap the pencil and drag the Kova tile in.

## Updates and backups

### How do I update Kova and the cores?
**Settings → Updates → Check now**, then **Install and restart** (Android: **Download update**). Cores: **Settings → Cores → Check for updates**, then **Update to...** (on Android cores update with the app). Turn on **Get beta versions** for pre-releases.

### How do I back up and restore?
**Settings → App → Backup → Save...** writes all profiles and settings to one file; add a password to encrypt it. **Restore...** reads it back, on this or another device. The file contains your subscriptions, so keep it private.

### How do I reset Kova?
Quit Kova and delete the data folder (see where above). It then starts as new. To undo only appearance, use **Settings → Look → Reset look**. Not confirmed: a single in-app reset button.

### How do I uninstall?
Use the system's uninstall (Windows installed apps, Android app info, your package manager). Uninstalling Kova removes the Windows service. Whether the data folder is deleted too is not confirmed; delete it by hand to remove all data. If the system proxy is left set after a crash, Kova restores it at the next start.
