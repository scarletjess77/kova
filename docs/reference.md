# Kova options reference

This page lists what you can see and change in Kova, page by page, in the order of the sidebar: Dashboard, Proxies, Profiles, Connections, Rules, Site check, Logs, Settings. After that come the tray, the Windows service and a few features that cut across pages.

Marks used below:

- **Default** is the value on a new install.
- **Desktop only** and **Android only** are platform limits. Where nothing is said, the option exists everywhere.
- **Reconnect**: Kova restarts a running connection by itself when the change needs it. You do not need to do anything.
- **(new in 0.3.0)** means the feature is not in a released version yet.

Names in bold are the exact texts in the app. Language is English here; the app can also show Russian, Persian and Chinese.

## Window and title bar

- **Edit layout** (pencil in the title bar): starts layout editing. See "Layout presets and Edit layout" under Dashboard.
- **Donate**: opens the support dialog. Support is voluntary.
- **FAQ** (question-mark icon in the title bar): opens the FAQ page in your browser.
- **Theme** (moon icon): opens the theme picker (see Settings, App).
- **Close to tray** (the close button, desktop only): hides the window. Kova keeps running. Use **Quit Kova** to end it.
- **Menu** (Android phone): opens the sidebar.

The bar at the bottom (desktop) shows six items: connection state, **Port**, capture (TUN, System proxy, or "Proxy port only"), routing mode, download speed and upload speed. You can hide each one while editing the layout.

## Dashboard

The first page. It is made of cards you can reorder and hide. A banner above the cards warns when the active subscription has expired, ends in 3 days or less, or has used 90 percent of its traffic, and shows an announcement from the provider if the subscription carries one.

### What's wrong?

Button at the top right (an icon on a phone). It checks the connection step by step: a profile exists, the cores are installed, the network works without Kova, Kova is connected, the server answers, names resolve, sites open through Kova, apps are sent through Kova, and what address sites see. Each failed step has a button that fixes it where possible (add a profile, open Settings, connect, choose a server, use Rule mode, turn on System proxy, turn on IPv6).

- **Copy report** puts a text report on the clipboard. See the FAQ for what is in it.
- **Check again** repeats the checks.
- Uses `ya.ru` to see whether the network itself works.

### Connect and status

- The big power button connects and disconnects. It is disabled until a profile exists.
- **Status** shows **Disconnected**, **Connecting...**, **Switching...**, **Protected** or **Proxy only**. **Proxy only** means Kova runs but nothing captures your apps (no System proxy and no TUN). Apps then use Kova only if you point them at the proxy port by hand.
- The route line shows the chain of groups ending at the server in use, with its protocol and transport.
- Chips show the core in use (**Mihomo**, **Xray** or "Mihomo + Xray bridge"), **Anti-DPI** when it is active, and how long you have been connected.
- Warnings appear as chips: a core warning, "Apps aren't routed yet", and "n nodes skipped" when servers in the profile could not be used.
- Before you have a profile, the card shows a short guide. **Where are you?** offers **Russia**, **Iran**, **China** or **Elsewhere**. It switches the matching local-sites preset (see Rules). **Add profile** opens the import dialog.
- **Capture traffic** (desktop only), two switches:
  - **System proxy** (default on): sets the Windows, macOS or Linux system proxy to Kova's port. Covers browsers and most apps. Kova puts the previous proxy back when you disconnect or quit, and after a crash.
  - **TUN mode** (default off): captures all apps, games and UDP. Needs the Kova service on Windows (see TUN mode). Reconnect.
- On Android there is no choice: the VPN captures traffic. See Settings, VPN (Android).

### Exit point

Shows the public IP address, location and network that sites see, whether it is measured through the proxy or without it, the latency to the server in use, and the DNS mode (**Fake-IP** or **Redir-host**) with the first resolver.

- **Check again** repeats the lookup. **Copy IP** copies the address.
- Uses `ipwho.is` (or `ipinfo.io`).

### Connection quality

**Check quality** sends one connection to the latency test address and then ten requests over it. It shows **Latency** (median and range), **Jitter**, **Loss** and **Connecting** time. Results are cleared when you connect or disconnect.

### Traffic

A live chart of download and upload speed.

### Quick switch

A short list for choosing a server without leaving the Dashboard: the groups of the main group (up to 6), then your starred servers (a small star marks them), then the fastest servers that have been tested, up to five servers in all. With more than five starred servers only starred ones are listed, up to eight.

- **Test latency** runs the latency test. Starred servers are listed before any test and stay on top; they are dimmed until they have a result, and a starred server that did not answer stays dimmed. Other servers appear only once tested, because nothing is guessed. The hint to run a test shows while no server has been tested.
- Stars are set in the server window or on **Proxies**; the card follows them at once.
- The row you click is selected only if the main group is a select group. If the group is **Fastest server** or **Failover** (an automatic group), the card says it chooses by itself.
- **All servers** opens the server window (the full list with search, **Sort** and **Favorites**; **Manage in Proxies** leads to the Proxies page). The window remembers the **Sort** choice and the **Favorites** filter for next time, separately from the Proxies page; the search text is not kept.

### Session numbers

Three tiles: bytes downloaded and uploaded this session, **Connections** open now, and **Core memory** (Mihomo only; Xray does not report it).

### More details

A divider card. Cards below it stay behind the button **Exit point, traffic and more** (**Fewer details** closes them again).

### Routing mode and core

Hidden in the **Standard** layout on desktop and phone; shown in **Everything**.

- **Routing mode**, default **Rule**:
  - **Rule**: your rules, presets and the profile's rules decide for each site.
  - **Global**: everything goes through the proxy; rules are skipped.
  - **Direct**: everything goes direct; rules are skipped, no server is used.
  - Reconnect.
- **Core** (needs a profile): **Automatic**, **Mihomo** or **Xray**. Same as **Run on core** in Profiles. It changes the core of the active profile. On Android **Xray** is not offered unless the profile already uses it, because the VPN needs Mihomo in front.

### Layout presets and Edit layout

Also under Settings, App, **Layout**. Three presets:

| Preset | Tabs | Dashboard cards |
|---|---|---|
| **Standard** (default) | all | everything except **Routing mode and core** |
| **Simple** | Proxies and Profiles only (plus Dashboard and Settings) | Connect and status, Exit point and Quick switch. The status bar hides **Port** and routing mode; the Proxies list hides the "which cores" line |
| **Everything** | all | all cards |

**Edit layout** lets you drag the handle to move a tab or card and press the eye to show or hide it. This also works for the bottom status bar items and, for tabs, in the sidebar. **Connect and status** and the **More details** divider cannot be hidden. Dashboard and Settings always stay. Changes save as you go; **Done** ends editing.

Phones and computers keep separate layouts. A hidden tab still opens from links. Layouts are included in backups and theme files.

## Proxies

Lists the groups and servers of the active profile.

- The header shows the number of groups and nodes.
- **Search nodes, countries, protocols** filters the list by name, protocol, transport, server address or country code.
- **Sort**: **Profile order** (default), **Sort by latency**, **Sort by name**. Starred servers always come first in every order. The choice is remembered.
- **Favorites** (new in 0.3.0): shows only starred servers. Press the star on a server to add or remove it. Stars belong to the profile in use: the same name in another profile is another server. Stars are saved with the settings and included in backups.
- **View** menu (stored with the layout, per device):
  - **Compact rows**: one line per server. Turns the next four off visually.
  - **Protocol** (default on).
  - **Transport and security** (default on).
  - **Which cores run it** (default on): shows Mihomo and Xray dots and "via Xray bridge" when the server is bridged.
  - **Server address** (new in 0.3.0, default off): shows `host:port` on each server.
- **Test latency**: tests every server of the selected group, through each server, using the **Latency test address** from Settings, Servers.
- **Groups** on the left (or the top on a narrow screen). Each shows its type chip and the server it ends on.
  - **select**: you choose the member. Click a server to pin it.
  - **url-test** (shown as **Fastest server**): re-tests every few minutes and moves to the fastest member. See **Switch to a faster server**.
  - **fallback** (**Failover**): uses the first member that passes health checks.
  - **load-balance**: spreads connections across members.
  - **relay**: a chain.
  - Automatic groups are marked **Automatic**. Clicking a server in them only shows a note.
  - The **GLOBAL** group is listed only in Global mode.
- Each real server has two buttons beside it: the star (**Add to favorites**) and the eye (**Show configuration**, new in 0.3.0).
- **Show configuration** opens a read-only view of the server: protocol, server, port, transport, TLS or Reality settings, flow, and so on, plus a **Raw configuration** section and **Copy share link** when a link exists. Secrets (passwords, UUIDs, keys) are hidden until you press the eye on that row. Anyone who sees them can use the server.
- A server with no latency shows no number. See the FAQ.

## Profiles

A profile is one source of servers: a subscription address or a local config.

### Add profile

The dialog has tabs:

- **Link**: paste a subscription address (`http://` or `https://`) or share links, one per line.
- **Clipboard**: Kova reads the clipboard and tells you what it found. **Check again** reloads it.
- **QR code** (Android phone only): scans a code with the camera.
- **File** (desktop only): `.yaml`, `.yml`, `.json`, `.txt` or `.conf`; the file must be a regular local file of at most 8 MB.
- **Another device**: shows a QR code and an address on your local network. A phone on the same Wi-Fi opens the page, pastes the subscription, and Kova adds it. The page works once and for 10 minutes. It is the default tab on Android TV.
- **Name** (optional): taken from the subscription if empty.

It understands Clash/Mihomo YAML, sing-box JSON, Xray JSON, WireGuard `.conf`, base64 subscriptions, and share links of `vless`, `vmess`, `trojan`, `ss`, `hysteria2` (`hy2`), `tuic`, `wireguard`, `anytls`, `socks` and `naive+https`. Kova also opens `kova://` links, which fill in this dialog; you still press **Import**.

A subscription must be at most 20 MB. When it comes from a link, Kova takes the name and update interval from the provider if given (default 12 hours).

### Profile list

- **Update all** (shown when a subscription exists): updates every subscription.
- **All profiles** (shown with two or more profiles): the combined list. The radio button next to a profile makes it the one **In use**.
- Each row shows the kind (**Subscription** or **Local**), format, node count, last update, and a traffic bar when the provider reports usage. A chip warns when it expires in 14 days or less.

### A profile's settings

- **Name**.
- **Subscription URL** (subscriptions only).
- **Run on core** (default **Automatic**): **Automatic** runs Mihomo and bridges nodes only Xray supports (such as XHTTP) through a local Xray. **Mihomo only** and **Xray only** pin it. Xray only has no live connections list and no TUN. On Android **Xray only** is hidden unless already set.
- **Auto-update** (subscriptions; default 12 hours, or the provider's value): **Off**, every 1, 3, 6, 12 or 24 hours, every 2 days, every 3 days.
- **Update through the proxy** (subscriptions; default on): fetches the subscription through Kova when Kova is connected. It helps when the provider's site is blocked. Used only while connected. If one way fails, Kova tries the other.
- **User-Agent** (subscriptions; default empty, meaning `clash.meta`): providers pick the output format from it. The default asks for a Clash/Mihomo config. An Xray app's name such as `v2rayNG/1.10.0` often returns Xray JSON. The list suggests `v2rayNG/1.10.0`, `Happ/3.0.0`, `sing-box/1.12.0` and `clash.meta`.
- **Extra headers** (subscriptions): one `Name: value` per line, sent with every update. Only if the provider asks. Values can be access keys; they are stored sealed.
- **From the provider**: an announcement the provider sent.
- A **n entries skipped or changed** note lists server entries Kova could not read or had to change.
- Buttons: **Use this profile**, **Update now**, **Copy URL**, **Share**, **Edit config**, **Provider** (opens the provider's page when it gave one), **Delete** (asks to confirm).
- **Share**: shows a QR code when the text is short enough (up to 2000 characters) and a copy button. Config files (Clash, sing-box, Xray) are shared as text only. **Warning:** anyone with the link can use the subscription and its traffic.
- **Edit config**: edits the raw config (`config.yaml`, `config.json`, `wireguard.conf` or `links.txt`). Kova validates it with Mihomo before saving. Ports, the controller and TUN are managed by Kova and override your values. For a subscription, the next update replaces your edits.

### All profiles (combined list)

Puts the servers of several profiles in one list with Kova's own groups and rules. Tick the profiles to include (at least one must stay). Clash profiles contribute only their servers. If two servers share a name, the profile name is added to the second. Choose it with **Use all profiles together**.

## Connections

Needs a running connection and Mihomo. With an Xray-only profile the page says live connections are not available.

- **Filter by host, app or rule** searches the table.
- **Show**: **All**, **Proxied**, **Direct**.
- **Pause** and **Resume** stop the refresh (every second).
- **Close all** closes every open connection.
- Columns: **Host**, **App**, **Rule**, **Route**, download, upload, **Total**, **Time**. The list is sorted by current speed.
- Per row: a button to **Always send {host} direct** or **through the proxy**, which adds a rule at the top of your rules and closes the connection; and an x to close that connection.

## Rules

The first matching rule decides where traffic goes. The order is: your rules, torrent clients, presets, the local network (direct), then everything else uses the **Proxy** group (or **DIRECT** with the **Only blocked sites through the proxy** preset). **Global** and **Direct** modes skip rules.

**Apply bar.** Edits on this page and in **Route an app** wait. A bar appears with **Apply** and **Discard**. **Apply** applies everything at once and restarts a running connection once.

### Routing presets

A grid of compact tiles, five presets side by side where the window is wide (fewer per row in a narrow one). A closed tile is one line: an arrow, the title and the switch at the end. The explanation of a tile is hidden by default; press its arrow (or the title) to show or hide it, and only that tile grows. Each tile remembers on this device whether it is open. The same arrows are on **Torrent clients**, **Apply to imported Clash profiles**, **Smart routing** and **Also correct the subscription's rules**. Warnings and notices stay visible under their tile or row even when it is closed.

Switches. **Default**: on a first start Kova turns on **Block ads and trackers** and, from the system language, the local preset: Russian gives **Direct for Russian sites**, Chinese **Direct for Chinese sites**, Persian **Direct for Iranian sites**. Elsewhere no local preset.

- **Only blocked sites through the proxy**: sites blocked in Russia use the proxy; everything else goes direct. Needs the Russia routing data (Settings, Routing data); until then the switch is disabled (its tooltip says why), and the link to Settings is inside the tile's description.
- **Direct for Russian sites**: `.ru`, `.xn--p1ai` (.rf), `.su`, Russian services and IP ranges skip the proxy.
- **Block ads and trackers**: rejects the `category-ads-all` list. Risk: a site may break if it depends on a blocked tracker.
- **Direct for Chinese sites**: `geosite:cn` and Chinese IP ranges go direct.
- **Direct for Iranian sites**: `.ir` domains and Iranian IP ranges go direct.

### Torrent clients (new in 0.3.0, desktop only)

A wider tile in the same grid (two columns where there is room), desktop only. Choose one on the title line: **Not set** (default), **Through the proxy**, **Direct**, **Block** (in a narrow window the choices move under the title). It matches programs by name (qBittorrent, uTorrent, BitTorrent, Transmission, Deluge, KTorrent, rTorrent, PicoTorrent, Tribler and others). It works only for programs whose traffic goes through Kova: with TUN, with the system proxy, or when the program is pointed at Kova's port. Warnings: for an imported Clash profile it needs **Apply to imported Clash profiles**; many providers forbid torrents, so **Through the proxy** can get your account blocked.

### Your rules

Shown **checked before presets**. Add a rule with **Match type**, **Value** and **Send to**, then **Add rule**. A new rule goes to the top.

Match types: **Domain and subdomains** (`example.org`), **Exact domain**, **Domain contains**, **Geosite list** (a list name such as `youtube`), **Country (GeoIP)** (a code such as `DE`), **IP range** (CIDR such as `203.0.113.0/24`) and **App (process name)** (`steam.exe`).

Targets: **Proxy (main group)**, **DIRECT**, **REJECT** (block), **Fastest in {country}** (one entry per country your servers are in) and any group of the profile.

Reorder by dragging the grip or with the arrow keys; rules higher up win. The x deletes a rule.

- **ChatGPT, Claude, Gemini and other AI services through** a country: one click adds a Geosite rule for `category-ai-!cn` toward the fastest server of the chosen country. Shown while the rule does not exist.
- **Route an app...** (desktop only): lists programs using the network now, programs already routed, and other running programs; **Choose a program...** adds one from disk. Each row has **Proxy**, **Direct** and **Block**, and an x to remove. These rules go above all others. With the system proxy only programs that use it are affected; TUN covers every program. On Windows each row shows the program's icon when it has one; icons are cached in the `icons` folder of the data folder (up to 300 files), so a program that is not running now keeps its last icon.
- **Apply to imported Clash profiles** (default on): puts your rules and presets in front of an imported Clash profile's own rules. Off, a Clash profile keeps only its own rules.

### Smart routing (beta)

Switch, default off. Kova notices sites that fail on their usual path, checks them directly and through the proxy, and sends each the way that works. With TUN it also learns apps that connect by IP address. Learned sites are kept for 30 days. It works in **Rule** mode only. In TUN mode keep **Sniff domains** or **Fake-IP** on so Kova sees site names. With a Clash profile it also needs **Apply to imported Clash profiles**.

- **Also correct the subscription's rules** (Clash profiles; default off): also fixes sites the subscription's own rules send the wrong way. Your own rules and presets are never changed.
- **Send sites that refuse the proxy direct** (default off): lets Kova open such a site directly to test it. **Risk:** once sent direct the site sees your real address. With it off, Kova only moves blocked sites to the proxy.
- **Suggestions**: when a rule you chose on purpose sends a site somewhere it fails, Kova offers **Add exception** or **Dismiss**. A badge on the Rules tab counts waiting suggestions.
- **Learned** list: **Keep** turns a learned site into one of your rules for good; the x forgets it; **Forget all** clears the list.

## Site check

Checks whether sites can be reached with Kova and without it.

- **Add** a site of your own (**Your sites**); **Check all** tests every site, six at a time. Each site has a refresh button, and your own sites an x.
- Built-in groups: **Baseline**, **Video**, **Social**, **Messengers**, **AI**, **Other**.
- Results: **Reachable**, **Unblocked by Kova**, **Blocked**, **Region-locked** (the site refused access), **Unreachable**. Failures say what happened (timeout, name not resolved, connection reset, broken secure connection).
- When Kova is not connected only direct access is checked. With TUN on, "Without Kova" also goes through Kova and shows the same result.
- **Privacy check**: your address with and without Kova, whether DNS lookups are encrypted, IPv6 leaks, and whether all apps go through Kova. Uses `api.ipify.org`.
- **Speed test**: downloads a test file from Cloudflare for a few seconds, through Kova and without it. Uses `speed.cloudflare.com`.

## Logs

- Live lines from the core and from Kova, while connected. Only the current run is shown; the list keeps the newest lines.
- **Filter messages**, **Minimum level** (**Debug**, **Info** (default), **Warning**, **Error**), **Pause**/**Resume**, **Clear**.
- **Copy report**: the same support report as in **What's wrong?**.
- To see debug lines, also set **Core log level** to **Debug** in Settings.

## Settings

Changes apply right away. A running connection restarts when needed. Each card can be folded.

### Network

- **Proxy port (HTTP + SOCKS5)**, default 7890. Allowed 1024 to 65534. Xray profiles also use the port above it, so 65535 is out. Reconnect.
- **Skip the system proxy for** (desktop only): a list of addresses that bypass the system proxy. Default covers localhost, `127.*`, private ranges, `*.local` and `<local>`.
- **Allow connections from LAN** (default off): shares the proxy with other devices. Kova shows the address and a password; other devices sign in as `kova`. With an Xray core the LAN uses two ports above the proxy port (HTTP and SOCKS5). **Risk:** anyone on your network with the password can use your connection.
- **Switch away from a dead node** (default on): when the chosen node stops answering, moves to the fastest one that works and says so.
- **Switch to a faster server** (new in 0.3.0): default **if faster by 50 ms**; the options read **if faster by 10 ms**, **if faster by 25 ms**, **if faster by 50 ms**, **if faster by 100 ms**, **if faster by 200 ms**, **Never**. The **Fastest server** group stays on its server unless another is faster by more than this. **Never** keeps the server until it stops answering. Lower follows the fastest closely; higher avoids switching.
- **IPv6** (default on). With it off, IPv6 traffic can go around TUN; the privacy check warns about that.
- **Sniff domains** (default on): reads TLS and HTTP hostnames so domain rules also work for connections made by IP.
- **Kill switch** (beta, desktop only, default off): if the connection drops on its own, apps that use the system proxy go offline instead of around Kova, and Kova reconnects. **Disconnect** lifts it. In TUN-only mode Kova just reconnects. It does not hold programs that ignore the system proxy.
- **Anti-DPI** (beta, default off): sends the start of each connection in small pieces so simpler blocking equipment can't recognise it. Needs the Xray core. Helps when servers stop connecting; does not help against whitelists. Reconnect. Every server connection is dialled through a local Xray while it is on.

### Routing data

Source of the GeoIP and GeoSite lists that rules use. Choose **Standard** (MetaCubeX lists, updated weekly, default), **Russia** (adds sites blocked in Russia, about 90 MB, updated every 12 hours; needed for **Only blocked sites through the proxy**) or **Custom** (two direct links, **geoip.dat address** and **geosite.dat address**, then **Download and use**; Kova checks the files can be read but cannot verify who published them). **Update now** refreshes. The card shows what is installed, whether the checksum was verified and any list your presets need that is missing. Uses `github.com`.

### Cores

Shows **Mihomo**, **Xray** and **Shoes** with their installed versions. **Check for updates** (desktop) looks for newer ones; **Install** or **Update to {version}** installs it after a SHA-256 check. Shoes runs NaiveProxy servers on Linux and macOS and is installed when a profile needs it. On Android the cores come with the app.

### Updates

- **Check for updates on launch** (default on). Releases are signed; Kova installs only updates signed with its key.
- **Get beta versions** (default on only in a beta build): updates to whichever is newer, beta or stable.
- **Check now**, then **Install and restart** (Windows asks for administrator approval) or **Download update** (Android asks to confirm).

### App

- **Launch at sign-in** (desktop only, default off).
- **Start in the tray** (desktop only, default off).
- **Connect on launch** (default off).
- **Backup**: **Save...** and **Restore...**. See Backup and restore below.
- **Theme**: **Match system**, **Dark** (default), **Light**, **OLED black**, **Dim**, **Warm paper**, **High contrast**, **Night sky**, **Nord**, **Catppuccin**, **Dracula**, **Gruvbox**, **Solarized**.
- **Layout**: presets and **Edit layout** (see Dashboard).
- **Language**: **Match system** (default), English, Russian, Persian, Chinese.
- **Core log level** (default **Info**): **Debug**, **Info**, **Warning**, **Error**, **Silent**.
- **Data folder** (desktop only): **Open** shows where Kova keeps its files.
- **Quit Kova** (desktop only): disconnects and restores your system proxy.

### Servers

- **Latency test address**: default `https://www.gstatic.com/generate_204`. Must start with `http://` or `https://`. Used by latency tests and automatic groups; change it if that address is blocked. Reconnect.
- **Hide servers in**: country codes read from server names. Hidden servers are left out of the list and of the groups.
- **Hide protocols**: leave out all servers of a protocol, for example hide everything but VLESS.

### VPN (Android only)

- **Apps through the VPN**: **All apps** (default), **Only chosen** or **All but chosen**, with search, **System apps**, **Copy list** and **Paste list** (package names). Saving reconnects.
- **Server on mobile data**: a server Kova uses on mobile data and leaves on Wi-Fi. Default **Same as on Wi-Fi**. Meant for networks that only allow whitelisted sites.
- **Always-on VPN and kill switch**: opens Android's VPN settings. Turn on Always-on VPN, and **Block connections without VPN** to keep apps offline when Kova is not connected.
- **Full screen** (not on TV), **Keep running in the background** (battery exemption), **Autostart** (shown on some makers' phones) and **Quick Settings tile** (instructions).

### TUN mode (desktop only)

The switch turns TUN on. **Reconnect.**

- **Network stack**: **Mixed** (default), **System**, **gVisor**.
- **Set up routes automatically** (default on).
- **Strict route** (default on): stops DNS and traffic from leaking around the tunnel.
- **Kova service** (Windows): shows whether it is running and its version. **Install** installs it (administrator approval once). **Repair or update** re-installs it when it is not running or comes from another Kova build. **Remove** uninstalls it.
- Linux: **Give rights** grants network rights to Kova's copy of Mihomo once (asks for your password). **Give again** after a Mihomo update.
- macOS: TUN needs Kova started with `sudo`. The system proxy works without it.

### DNS

- **Fake-IP** (default on): fastest. Turn off if an app needs real IP answers.
- **Resolvers for proxied traffic**: default `https://1.1.1.1/dns-query`, `https://dns.google/dns-query`. Comma separated; encrypted (https) resolvers keep your network from reading lookups.
- **Resolvers for direct traffic and server addresses**: default `https://common.dot.dns.yandex.net/dns-query`, `77.88.8.8`.
- **Use these settings for Clash profiles too** (default off): otherwise a Clash profile's own DNS section is kept.

### Look

Drawn over the theme. Changes show at once.

- **Import** and **Export** a `.kovatheme` file (colours, look, layouts, pictures, custom CSS; never profiles or settings). **Reset look** clears everything here and deletes the kept pictures.
- **Colours**: **Accent**, **Background**, **Cards**, **Text**. Pick or type a code like `#1e90ff`. If a text colour is unreadable on the cards, Kova uses black or white instead.
- **Font**: Onest (default), **System font**, Vazirmatn, JetBrains Mono, Pixelify Sans.
- **Density**: **Comfortable** or **Compact**.
- **Country shown as**: **Code** or **Flag**.
- **Size**, **Corners**, **Card transparency** sliders.
- **Pictures**: pick a place (**Behind every tab**, **Sidebar** (desktop), **Connect card**, or behind one tab), then **Choose picture**. PNG, JPG, WebP or animated GIF up to 10 MB; Kova keeps its own copy. **Fit** (**Fill**, **Whole picture**, **Tile**), **Position**, **Dim**, **Blur** (backgrounds only). **Save template** (desktop) saves a guide image.
- **Custom CSS** (advanced): after **Apply**, a bar asks to **Keep** it and undoes it after 15 seconds otherwise. On a computer Ctrl+Shift+K turns it off until Kova restarts.
- **Card outlines** (default on).

### Support Kova

The donation dialog and credits. Voluntary.

### What Kova connects to

Lists every outside address Kova uses. See Privacy below.

## Tray (desktop)

Left-click the tray icon to open the window. The menu has **Connect** or **Disconnect**, **Server** (the servers of the main group, up to 40), **Profile** (up to 40), **Routing mode** (**Rule**, **Global**, **Direct**), **System proxy**, **TUN mode**, **Open Kova** and **Quit Kova**. The tooltip says whether Kova is connected. Starting Kova with `--minimized` or turning on **Start in the tray** keeps the window hidden.

## The Kova service (Windows)

A small Windows service (`KovaService`, started automatically with Windows) that starts Mihomo with the rights TUN needs, so Kova itself never runs as administrator. It runs only its own copy of Mihomo, stops it when the app that started it exits, and accepts only what Kova needs from it. Install, **Repair or update** and **Remove** are in Settings, TUN mode. The Windows installer registers it. Details: [how-it-works.md](how-it-works.md#tun-mode-and-the-kova-service).

## Kill switch, Anti-DPI, smart routing

All three are beta and off by default. See Settings, Network, and Rules, Smart routing, above.

## Backup and restore

Settings, App, **Backup**. **Save...** writes one `.json` file with every profile and the settings. The file holds your subscriptions: anyone who has it can use them. An optional password (asked twice) encrypts it. **Restore...** opens a file, asks for the password if needed, shows the profiles inside and the Kova version that made it, and offers **Also restore the settings: theme, rules, DNS and the rest**. Profiles that are already present are skipped. A backup from a newer Kova is refused.

## Languages, themes, pictures

Four languages: English, Russian, Persian (right to left) and Chinese. Twelve themes plus **Match system**. Pictures and colours are under Settings, Look. A theme file can carry all of them.

## What Kova connects to

Besides your servers:

| Address | For |
|---|---|
| `github.com` | Kova updates, cores, routing data |
| Your subscription's address | Updating a profile. Kova sends a random device ID made for this install (not a hardware ID); providers use it to count devices |
| `www.gstatic.com` (or your **Latency test address**) | Latency tests and health checks, through your servers |
| `ipwho.is`, `ipinfo.io` | The **Exit point** card |
| `ya.ru` | **What's wrong?** |
| `api.ipify.org` | **Privacy check** |
| `speed.cloudflare.com` | **Speed test** |
| The sites in **Site check** | Only when you run it |

No accounts, ads or telemetry.
