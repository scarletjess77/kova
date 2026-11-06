<script lang="ts">
  import { disable as autostartOff, enable as autostartOn, isEnabled as autostartIsOn } from '@tauri-apps/plugin-autostart';
  import { getVersion } from '@tauri-apps/api/app';
  import { openPath } from '@tauri-apps/plugin-opener';
  import { api, errText, type CoreKind, type CoreStatus, type GeoInfo, type GeoSource, type ServiceView, type Settings } from '../lib/api';
  import { countryCode, fmtAgo, fmtBytes, notesHtml } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import AppPicker from '../components/AppPicker.svelte';
  import BackupModal from '../components/BackupModal.svelte';
  import Donate from '../components/Donate.svelte';
  import FoldCard from '../components/FoldCard.svelte';
  import Icon from '../components/Icon.svelte';
  import ThemePicker from '../components/ThemePicker.svelte';
  import ThemeFileModal from '../components/ThemeFileModal.svelte';
  import { PRESETS, presetOf, type Preset } from '../lib/layout';
  import { CSS_LIMIT, DEFAULT_RADIUS, FONTS, LIMITS, isEmpty, type FontId, type Look, type LookColors } from '../lib/look';
  import { DEFAULT_DIM, TAB_VIEWS, isBackdrop, keepPicture, pictureStyle, removePicture, saveChosenFile, slotSize, templatePng, type Fit, type Picture, type Pos, type SlotId } from '../lib/pictures';
  import { save as saveDialog } from '@tauri-apps/plugin-dialog';

  let appVersion = $state('');
  // Members of the group traffic starts in ("Proxy", or an imported config's first
  // selector), for the server used on mobile data.
  let mainServers = $derived(
    (app.proxies.groups.find((g) => g.name === 'Proxy') ?? app.proxies.groups.find((g) => g.type === 'Selector' && g.name !== 'GLOBAL'))?.all ?? [],
  );
  let pickingApps = $state(false);
  let backup = $state<'save' | 'restore' | null>(null);
  // Makers whose phones stop apps beyond Android's own battery saving.
  const OWN_AUTOSTART = ['xiaomi', 'redmi', 'poco', 'huawei', 'honor', 'oppo', 'realme', 'oneplus', 'vivo', 'iqoo'];
  let lanIp = $state<string | null>(null);
  $effect(() => { if (app.settings?.core.allow_lan) void api.lanAddress().then((v) => (lanIp = v), () => {}); });
  getVersion().then((v) => (appVersion = v), () => {});

  let s = $derived(app.settings);
  let cores = $derived(app.cores);
  let checking = $state(false);
  let installing = $state<CoreKind | null>(null);
  const CORE_NAMES: Record<CoreKind, string> = { mihomo: 'Mihomo', xray: 'Xray', shoes: 'Shoes' };

  async function loadCores(latest: boolean) {
    checking = latest;
    const got = await app.run(() => api.coreStatus(latest));
    if (got) app.cores = got;
    checking = false;
  }
  // The installed versions come at once from disk; the latest ones from GitHub after.
  $effect(() => {
    void app.coresVersion;
    void (async () => {
      if (!app.cores.length) await loadCores(false);
      await loadCores(true);
    })();
  });

  const norm = (v: string | null) => (v ?? '').replace(/^v/, '');
  async function install(c: CoreKind) {
    installing = c;
    await app.run(() => api.installCore(c), (tag) => t('{core} {tag} installed', { core: CORE_NAMES[c], tag }));
    installing = null;
    await loadCores(false);
  }

  const set = (f: (x: Settings) => void, ok?: string) => app.saveSettings(f, ok);
  let PRESET_NAME = $derived<Record<Preset, string>>({ standard: t('Standard'), simple: t('Simple'), everything: t('Everything') });
  let layoutPreset = $derived(presetOf(app.layout, app.form));

  // The look over the theme. Colours the user didn't pick show the theme's own.
  let look = $derived<Look>(app.settings?.look ?? {});
  let COLOR_NAME = $derived<Record<keyof LookColors, string>>({ accent: t('Accent'), bg: t('Background'), panel: t('Cards'), fg: t('Text') });
  const COLOR_KEYS: (keyof LookColors)[] = ['accent', 'bg', 'panel', 'fg'];
  const COLOR_VAR: Record<keyof LookColors, string> = { accent: '--accent', bg: '--win', panel: '--panel', fg: '--fg' };
  let FONT_NAME = $derived<Record<FontId, string>>({ onest: 'Onest', system: t('System font'), vazir: 'Vazirmatn', mono: 'JetBrains Mono', pixel: 'Pixelify Sans' });
  /** The colour shown in a swatch: the user's, or the theme's. */
  function swatch(k: keyof LookColors): string {
    void app.settings?.theme;
    const own = look.colors?.[k];
    if (own) return own;
    const probe = document.createElement('div');
    probe.style.color = `var(${COLOR_VAR[k]})`;
    document.body.appendChild(probe);
    const rgb = getComputedStyle(probe).color.match(/\d+/g)?.slice(0, 3).map(Number) ?? [0, 0, 0];
    probe.remove();
    return '#' + rgb.map((v) => v.toString(16).padStart(2, '0')).join('');
  }
  let themeFile = $state<'export' | 'import' | null>(null);

  // Custom CSS: written here, tried with a countdown, kept on request.
  let cssDraft = $state(app.settings?.look?.css ?? '');
  let cssOpen = $state(!!app.settings?.look?.css);
  function applyCss() {
    if (cssDraft.length > CSS_LIMIT) { app.notify(t('Custom CSS is limited to {n} characters', { n: CSS_LIMIT })); return; }
    app.tryCss(cssDraft);
  }

  // Pictures: one place chosen at a time.
  const VIEW_NAME: Record<string, () => string> = {
    dashboard: () => t('Dashboard'), proxies: () => t('Proxies'), profiles: () => t('Profiles'), connections: () => t('Connections'),
    rules: () => t('Rules'), check: () => t('Site check'), logs: () => t('Logs'), settings: () => t('Settings'),
  };
  let SLOTS = $derived<[SlotId, string][]>([
    [app.mobile ? 'app-phone' : 'app', t('Behind every tab')],
    ...(app.mobile ? [] : [['sidebar', t('Sidebar')] as [SlotId, string]]),
    ['banner', t('Connect card')],
    ...TAB_VIEWS.map((v) => [`tab-${v}`, t('Behind {tab}', { tab: VIEW_NAME[v]() })] as [SlotId, string]),
  ]);
  let slot = $state<SlotId>('app');
  $effect(() => { if (!SLOTS.some(([id]) => id === slot)) slot = SLOTS[0][0]; });
  let slotName = $derived(SLOTS.find(([id]) => id === slot)?.[1] ?? '');
  let pic = $derived<Picture | undefined>(look.pictures?.[slot]);
  let size = $derived(slotSize(slot));
  let picUrl = $derived(app.pictureUrl(pic?.file));
  let picking = $state(false);
  let picInput = $state<HTMLInputElement>();
  const setPic = (patch: (p: Picture) => void, save = true) => {
    const f = (l: Look) => { const all = { ...(l.pictures ?? {}) }; const p = { ...all[slot]! }; patch(p); all[slot] = p; l.pictures = all; };
    if (save) void app.saveLook(f); else app.previewLook(f);
  };
  async function choosePicture(file: File | undefined) {
    if (!file) return;
    picking = true;
    const name = await app.run(() => keepPicture(slot, file));
    picking = false;
    if (picInput) picInput.value = '';
    if (name) void app.saveLook((l) => { l.pictures = { ...(l.pictures ?? {}), [slot]: { ...(l.pictures?.[slot] ?? {}), file: name } }; });
  }
  async function dropPicture() {
    await app.run(() => removePicture(slot));
    void app.saveLook((l) => { const all = { ...(l.pictures ?? {}) }; delete all[slot]; l.pictures = all; });
  }
  async function saveTemplate() {
    const [w, h] = slotSize(slot);
    const path = await saveDialog({ defaultPath: `kova-${slot}-${w}x${h}.png`, filters: [{ name: 'PNG', extensions: ['png'] }] });
    if (!path) return;
    const labels = { title: t('Title bar'), sidebar: t('Sidebar'), cards: t('Cards cover most of this; the picture shows around and between them'), top: t('Top bar'), tabs: t('Tabs'), profile: t('Profile card'), power: t('Power button'), status: t('Status'), controls: t('Mode and capture') };
    const bytes = await templatePng(slot, slotName, labels);
    await app.run(() => saveChosenFile(path, bytes), t('Template saved'));
  }

  const setColor = (k: keyof LookColors, v: string | undefined, save: boolean) => {
    const f = (l: Look) => { const c = { ...(l.colors ?? {}) }; if (v) c[k] = v; else delete c[k]; l.colors = c; };
    if (save) void app.saveLook(f); else app.previewLook(f);
  };


  // Countries of the active profile's servers, all of them (hidden ones included, which
  // the running core no longer has), for hiding a country.
  let profileCountries = $state<string[]>([]);
  let profileProtocols = $state<string[]>([]);
  $effect(() => {
    const id = app.settings?.active_profile;
    void app.settings?.combined_skip;
    if (!id) return;
    api.profileNodes(id).then((n) => {
      profileCountries = n.map((x) => countryCode(x.name)).filter((c): c is string => !!c);
      profileProtocols = n.map((x) => x.protocol);
    }, () => {});
  });
  // Protocols of the active profile's servers, hidden ones included, for hiding a protocol.
  let protocolList = $derived([...new Set([...profileProtocols, ...(s?.core.hidden_protocols ?? [])])].sort());
  const hideProtocol = (p: string) => set((x) => {
    const h = x.core.hidden_protocols;
    x.core.hidden_protocols = h.includes(p) ? h.filter((v) => v !== p) : [...h, p];
  });
  let countryList = $derived([...new Set([...profileCountries, ...(s?.core.hidden_countries ?? [])])].sort());
  const hideCountry = (c: string) => set((x) => {
    const h = x.core.hidden_countries;
    x.core.hidden_countries = h.includes(c) ? h.filter((v) => v !== c) : [...h, c];
  });
  function setTestUrl(v: string) {
    const u = v.trim();
    if (u && !/^https?:\/\/[^\s/]\S*$/.test(u)) { app.notify(t('Use an http:// or https:// address')); return; }
    void set((x) => (x.core.test_url = u), t('Saved'));
  }
  const toggle = (get: (x: Settings) => boolean, put: (x: Settings, v: boolean) => void) => set((x) => put(x, !get(x)));
  const list = (v: string) => v.split(/[,\n]/).map((x) => x.trim()).filter(Boolean);

  // Kova service (TUN without running the app as administrator)
  let svc = $state<ServiceView | null>(null);
  let svcBusy = $state<'install' | 'remove' | null>(null);
  const loadSvc = async () => { svc = (await app.run(() => api.serviceStatus())) ?? svc; };
  $effect(() => { void loadSvc(); });
  let svcOutdated = $derived(!!svc?.running && svc.outdated);
  async function installSvc() {
    svcBusy = 'install';
    const linux = app.platform === 'linux';
    await app.run(() => api.serviceInstall(), linux ? t('TUN mode can be used now') : svc?.installed ? t('Kova service updated') : t('Kova service installed. TUN mode no longer needs administrator rights'));
    svcBusy = null;
    await loadSvc();
  }
  async function removeSvc() {
    svcBusy = 'remove';
    await app.run(() => api.serviceUninstall(), app.platform === 'linux' ? t('Network rights taken back') : t('Kova service removed'));
    svcBusy = null;
    await loadSvc();
  }

  // Routing data (GeoIP / GeoSite)
  let geo = $state<GeoInfo | null>(null);
  let geoBusy = $state(false);
  let customOpen = $state(false);
  let draft = $state({ geoip: '', geosite: '' });
  const loadGeo = async () => { geo = (await app.run(() => api.geodataInfo())) ?? geo; };
  $effect(() => { void app.settings?.core.presets; void loadGeo(); });
  let geoKind = $derived(customOpen ? 'custom' : (s?.core.geodata.kind ?? 'default'));
  let GEO_LABEL = $derived<Record<string, string>>({ default: t('Standard (MetaCubeX)'), runetfreedom: t('Russia (runetfreedom)') });
  const geoSourceName = (id: string) => GEO_LABEL[id] ?? (id.startsWith('custom') ? t('Custom') : id);

  async function setGeo(source: GeoSource, ok: string) {
    geoBusy = true;
    await app.saveSettings((x) => (x.core.geodata = source), ok);
    geoBusy = false;
    customOpen = false;
    await loadGeo();
  }
  function pickGeo(kind: string) {
    if (kind === 'custom') {
      const cur = s?.core.geodata;
      draft = cur?.kind === 'custom' ? { geoip: cur.geoip, geosite: cur.geosite } : { geoip: '', geosite: '' };
      customOpen = true;
      return;
    }
    customOpen = false;
    if (kind !== s?.core.geodata.kind) void setGeo({ kind } as GeoSource, t('Routing data downloaded and checked'));
  }
  async function updateGeo() {
    geoBusy = true;
    await app.run(() => api.updateGeodata(), (m) => (m.verified ? t('Routing data updated and verified') : t('Routing data updated')));
    geoBusy = false;
    await loadGeo();
  }

  function port(v: string, put: (x: Settings, p: number) => void) {
    const p = Number(v);
    // Xray profiles also use the port above the proxy port, so 65535 itself is out.
    if (!Number.isInteger(p) || p < 1024 || p > 65534) { app.notify(t('Use a port between 1024 and 65534')); return; }
    void set((x) => put(x, p), t('Port saved'));
  }

  async function setAutostart() {
    if (!s) return;
    const on = !s.launch_at_login;
    try {
      await (on ? autostartOn() : autostartOff());
    } catch (e) {
      app.notify(errText(e));
      return;
    }
    void set((x) => (x.launch_at_login = on));
  }

  // The OS entry is the truth (it can be removed outside Kova); keep the switch in step.
  $effect(() => {
    if (app.mobile) return;
    autostartIsOn().then((on) => {
      if (app.settings && app.settings.launch_at_login !== on) void set((x) => (x.launch_at_login = on));
    }).catch(() => {});
  });
</script>

<div class="page-head"><h1>{t('Settings')}</h1><span class="sub">{t('Changes apply right away. A running connection restarts when needed.')}</span></div>

{#if s}
  <!-- The cards flow down one column and on into the next, in order, so folding one
       pulls the rest up and both columns stay about the same height. -->
  <div class="set">
      <FoldCard id="network" title={t('Network')}>
        <div class="form">
          <div class="fld full"><label for="st-port">{t('Proxy port (HTTP + SOCKS5)')}</label>
            <input id="st-port" class="input mono" value={s.core.mixed_port} onchange={(e) => port(e.currentTarget.value, (x, p) => (x.core.mixed_port = p))} /></div>
          {#if !app.mobile}
            <div class="fld full"><label for="st-bypass">{t('Skip the system proxy for')}</label>
              <input id="st-bypass" class="input mono small" value={s.proxy_bypass} onchange={(e) => set((x) => (x.proxy_bypass = e.currentTarget.value), t('Saved'))} /></div>
          {/if}
        </div>
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Allow connections from LAN')}</div><div class="d">{t('Share the proxy with phones and consoles on your network')}</div>
            {#if s.core.allow_lan}
              {#if lanIp && app.status.running && app.status.core === 'xray'}
                <!-- Xray can't let this computer in without the login, so the network gets its own two ports. -->
                <div class="d">{t('Address for other devices')} <span class="mono selectable">{lanIp}:{s.core.mixed_port + 2}</span> {t('(HTTP)')}, <span class="mono selectable">{lanIp}:{s.core.mixed_port + 3}</span> {t('(SOCKS5)')}</div>
              {:else if lanIp}<div class="d">{t('Address for other devices')} <span class="mono selectable">{lanIp}:{s.core.mixed_port}</span> {t('(HTTP and SOCKS5)')}</div>{/if}
              <div class="d">{t('Other devices sign in as {user} with the password', { user: 'kova' })} <span class="mono selectable">{s.core.local_password}</span></div>
            {/if}</div>
            <button class="switch" role="switch" aria-label={t('Allow connections from LAN')} aria-checked={s.core.allow_lan} onclick={() => toggle((x) => x.core.allow_lan, (x, v) => (x.core.allow_lan = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Switch away from a dead node')}</div><div class="d">{t('When the chosen node stops answering, move to the fastest one that works and say so')}</div></div>
            <button class="switch" role="switch" aria-label={t('Switch away from a dead node')} aria-checked={s.auto_switch} onclick={() => toggle((x) => x.auto_switch, (x, v) => (x.auto_switch = v))}></button></div>
          <div class="row-sw"><div><div class="t">IPv6</div></div>
            <button class="switch" role="switch" aria-label="IPv6" aria-checked={s.core.ipv6} onclick={() => toggle((x) => x.core.ipv6, (x, v) => (x.core.ipv6 = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Sniff domains')}</div><div class="d">{t('Reads TLS and HTTP hostnames so domain rules also work for raw IP connections')}</div></div>
            <button class="switch" role="switch" aria-label={t('Sniff domains')} aria-checked={s.core.sniff} onclick={() => toggle((x) => x.core.sniff, (x, v) => (x.core.sniff = v))}></button></div>
          {#if !app.mobile}
            <div class="row-sw"><div><div class="t">{t('Kill switch')} <span class="chip">beta</span></div><div class="d">{t('If the connection drops on its own, apps that use the system proxy go offline instead of around Kova, and Kova reconnects. Disconnect lifts it. In TUN-only mode Kova just reconnects.')}</div></div>
              <button class="switch" role="switch" aria-label={t('Kill switch')} aria-checked={s.kill_switch} onclick={() => toggle((x) => x.kill_switch, (x, v) => (x.kill_switch = v))}></button></div>
          {/if}
          <div class="row-sw"><div><div class="t">{t('Anti-DPI')} <span class="chip">beta</span></div><div class="d">{t('Sends the start of each connection to the server in small pieces, so simpler blocking equipment can’t recognise it. Try it when servers stop connecting; it doesn’t help against whitelists. Needs the Xray core')}</div></div>
            <button class="switch" role="switch" aria-label={t('Anti-DPI')} aria-checked={s.core.anti_dpi} onclick={() => toggle((x) => x.core.anti_dpi, (x, v) => (x.core.anti_dpi = v))}></button></div>
        </div>
      </FoldCard>

      <FoldCard id="servers" title={t('Servers')}>
        <div class="form">
          <div class="fld full"><label for="st-test">{t('Latency test address')}</label>
            <input id="st-test" class="input mono small" value={s.core.test_url} placeholder="https://www.gstatic.com/generate_204" onchange={(e) => setTestUrl(e.currentTarget.value)} /></div>
        </div>
        <div class="rows">
          <div class="t">{t('Hide servers in')}</div>
          <div class="d">{t('Servers whose names show one of these countries are left out.')}</div>
          {#if countryList.length}
            <div class="countries" role="group" aria-label={t('Hide servers in')}>
              {#each countryList as c (c)}
                <button class="chip mono" class:off={s.core.hidden_countries.includes(c)} aria-pressed={s.core.hidden_countries.includes(c)} onclick={() => hideCountry(c)}>{c}</button>
              {/each}
            </div>
          {:else}
            <div class="d">{t('No server names in this profile show a country.')}</div>
          {/if}
        </div>
        <div class="rows">
          <div class="t">{t('Hide protocols')}</div>
          <div class="d">{t('Servers of these protocols are left out. To keep only VLESS, hide the rest.')}</div>
          {#if protocolList.length}
            <div class="countries" role="group" aria-label={t('Hide protocols')}>
              {#each protocolList as p (p)}
                <button class="chip" class:off={s.core.hidden_protocols.includes(p)} aria-pressed={s.core.hidden_protocols.includes(p)} onclick={() => hideProtocol(p)}>{p}</button>
              {/each}
            </div>
          {/if}
        </div>
      </FoldCard>

      {#if app.mobile}
      <FoldCard id="vpn" title="VPN">
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Apps through the VPN')}</div>
            <div class="d">{s.vpn_apps.mode === 'all' ? t('All apps') : s.vpn_apps.mode === 'only'
              ? tn(s.vpn_apps.apps.length, 'Only {n} chosen app', 'Only {n} chosen apps')
              : tn(s.vpn_apps.apps.length, 'All but {n} chosen app', 'All but {n} chosen apps')}</div></div>
            <button class="btn" onclick={() => (pickingApps = true)}>{t('Choose')}</button></div>
          <div class="row-sw"><div><div class="t">{t('Server on mobile data')}</div><div class="d">{t('Mobile networks may only let through whitelisted sites. Choose a server that works there, such as one in Russia or behind a CDN; Kova switches to it on mobile data and back on Wi-Fi.')}</div></div>
            <select class="input mobile-server" aria-label={t('Server on mobile data')} value={s.mobile_server ?? ''}
              onchange={(e) => { const v = e.currentTarget.value; void app.saveSettings((x) => (x.mobile_server = v || null)); }}>
              <option value="">{t('Same as on Wi-Fi')}</option>
              {#each mainServers as n (n)}<option value={n}>{n}</option>{/each}
              {#if s.mobile_server && !mainServers.includes(s.mobile_server)}<option value={s.mobile_server}>{s.mobile_server}</option>{/if}
            </select></div>
          <div class="row-sw"><div><div class="t">{t('Always-on VPN and kill switch')}</div><div class="d">{t('Android starts Kova’s VPN by itself, after a restart too. Pick Kova in Android’s VPN settings and turn on Always-on VPN; also turn on Block connections without VPN to keep apps offline whenever Kova isn’t connected.')}</div></div>
            <button class="btn" onclick={() => app.run(() => api.openVpnSettings())}>{t('Open')}</button></div>
          {#if !app.tv}
            <div class="row-sw"><div><div class="t">{t('Keep running in the background')}</div>
              <div class="d">{app.batteryFree ? t('Battery saving leaves Kova alone.') : t('Android may stop Kova to save battery while the screen is off. Allow it to keep running.')}</div></div>
              {#if !app.batteryFree}<button class="btn" onclick={() => app.run(() => api.allowBackground())}>{t('Allow')}</button>{/if}</div>
            {#if OWN_AUTOSTART.includes(app.maker)}
              <div class="row-sw"><div><div class="t">{t('Autostart')}</div><div class="d">{t('Phones by this maker also stop apps on their own. Turn on autostart for Kova there.')}</div></div>
                <button class="btn" onclick={() => app.run(() => api.openAutostart())}>{t('Open')}</button></div>
            {/if}
          {/if}
          <div class="row-sw"><div><div class="t">{t('Quick Settings tile')}</div><div class="d">{t('Swipe down twice, tap the pencil and drag the Kova tile in to connect and disconnect from there.')}</div></div></div>
        </div>
      </FoldCard>
      {/if}
      {#if pickingApps}<AppPicker onclose={() => (pickingApps = false)} />{/if}

      {#if !app.mobile}
      <FoldCard id="tun" title={t('TUN mode')}>
        {#snippet aside()}<button class="switch aside-sw" role="switch" aria-label={t('TUN mode')} aria-checked={s.core.tun.enabled} onclick={() => toggle((x) => x.core.tun.enabled, (x, v) => (x.core.tun.enabled = v))}></button>{/snippet}
        <div class="fld"><span class="lbl">{t('Network stack')}</span>
          <div class="seg" role="group" aria-label={t('Network stack')}>
            {#each [['mixed', t('Mixed')], ['system', t('System')], ['gvisor', 'gVisor']] as [v, l] (v)}
              <button aria-pressed={s.core.tun.stack === v} onclick={() => set((x) => (x.core.tun.stack = v as Settings['core']['tun']['stack']))}>{l}</button>
            {/each}
          </div>
        </div>
        {#if app.platform === 'linux'}
          <p class="hint">{t('TUN captures every app, including games and UDP. It needs network rights: Kova asks for your password once and gives them to its copy of Mihomo only, so Kova itself never runs as root.')}</p>
        {:else if app.platform === 'macos'}
          <p class="hint">{t('TUN captures every app, including games and UDP. On macOS it needs administrator rights for now: start Kova with sudo to use it. The system proxy works without them.')}</p>
        {:else}
          <p class="hint">{t('TUN captures every app, including games and UDP. It needs system rights, which the Kova service provides so Kova itself never runs as administrator.')}</p>
        {/if}
        {#if svc?.supported}
          <div class="svc">
            <span class="led" class:on={svc.running}></span>
            <div class="svc-text">
              {#if app.platform === 'linux'}
                <div class="t">{t('Network rights for TUN')}</div>
                <div class="d">
                  {#if svc.installed}{t('Given')}{svcOutdated ? ` · ${t('Mihomo was updated since; give them again')}` : ''}
                  {:else}{t('Not given yet. Giving them asks for your password once.')}{/if}
                </div>
              {:else}
                <div class="t">{t('Kova service')}</div>
                <div class="d">
                  {#if svc.running}{t('Running · version {version}', { version: svc.version ?? '' })}{svc.core_version ? ` · Mihomo ${svc.core_version}` : ''}{svcOutdated ? ` · ${t('from another Kova build, update it')}` : ''}
                  {:else if svc.installed}{t('Installed but not running')}
                  {:else}{t('Not installed. Installing asks for administrator approval once.')}{/if}
                </div>
              {/if}
            </div>
            <div class="actions">
              {#if !svc.installed || !svc.running || svcOutdated}
                <button class="btn primary" class:busy={svcBusy === 'install'} disabled={svcBusy !== null} onclick={installSvc}>
                  {#if svcBusy === 'install'}<Icon name="refresh" />{t('Waiting for approval…')}{:else if app.platform === 'linux'}{svc.installed ? t('Give again') : t('Give rights')}{:else}{svc.installed ? t('Repair or update') : t('Install')}{/if}</button>
              {/if}
              {#if svc.installed}
                <button class="btn ghost danger" class:busy={svcBusy === 'remove'} disabled={svcBusy !== null} onclick={removeSvc}>{t('Remove')}</button>
              {/if}
            </div>
          </div>
        {/if}
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Set up routes automatically')}</div></div>
            <button class="switch" role="switch" aria-label={t('Set up routes automatically')} aria-checked={s.core.tun.auto_route} onclick={() => toggle((x) => x.core.tun.auto_route, (x, v) => (x.core.tun.auto_route = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Strict route')}</div><div class="d">{t('Stops DNS and traffic from leaking around the tunnel')}</div></div>
            <button class="switch" role="switch" aria-label={t('Strict route')} aria-checked={s.core.tun.strict_route} onclick={() => toggle((x) => x.core.tun.strict_route, (x, v) => (x.core.tun.strict_route = v))}></button></div>
        </div>
      </FoldCard>
      {/if}

      <FoldCard id="dns" title="DNS">
        <div class="row-sw first"><div><div class="t">Fake-IP</div><div class="d">{t('Fastest. Turn off if an app needs real IP answers.')}</div></div>
          <button class="switch" role="switch" aria-label="Fake-IP" aria-checked={s.core.dns.fake_ip} onclick={() => toggle((x) => x.core.dns.fake_ip, (x, v) => (x.core.dns.fake_ip = v))}></button></div>
        <div class="form">
          <div class="fld full"><label for="st-dns">{t('Resolvers for proxied traffic')}</label>
            <input id="st-dns" class="input mono small" value={s.core.dns.nameservers.join(', ')} onchange={(e) => set((x) => (x.core.dns.nameservers = list(e.currentTarget.value)), t('Saved'))} /></div>
          <div class="fld full"><label for="st-dns-d">{t('Resolvers for direct traffic and server addresses')}</label>
            <input id="st-dns-d" class="input mono small" value={s.core.dns.direct_nameservers.join(', ')} onchange={(e) => set((x) => (x.core.dns.direct_nameservers = list(e.currentTarget.value)), t('Saved'))} /></div>
        </div>
        <div class="row-sw"><div><div class="t">{t('Use these settings for Clash profiles too')}</div><div class="d">{t('Otherwise a Clash profile’s own DNS section is kept')}</div></div>
          <button class="switch" role="switch" aria-label={t('Use these settings for Clash profiles too')} aria-checked={s.core.dns.override_profile} onclick={() => toggle((x) => x.core.dns.override_profile, (x, v) => (x.core.dns.override_profile = v))}></button></div>
      </FoldCard>

      <FoldCard id="geodata" title={t('Routing data')}>
        {#snippet aside()}<span class="aside">{t('GeoIP and GeoSite lists used by rules')}</span>{/snippet}
        <div class="seg" role="group" aria-label={t('Routing data source')}>
          {#each [['default', t('Standard')], ['runetfreedom', t('Russia')], ['custom', t('Custom')]] as [v, l] (v)}
            <button aria-pressed={geoKind === v} disabled={geoBusy} onclick={() => pickGeo(v)}>{l}</button>
          {/each}
        </div>
        <p class="hint">
          {#if geoKind === 'default'}{t('MetaCubeX lists: every country and the v2fly site categories. Updated weekly.')}
          {:else if geoKind === 'runetfreedom'}{t('runetfreedom/russia-v2ray-rules-dat: the standard lists plus sites blocked in Russia. Needed for “Only blocked sites through the proxy”. About 90 MB, updated every 12 hours.')}
          {:else}{t('Direct links to')} <span class="mono">geoip.dat</span> {t('and')} <span class="mono">geosite.dat</span> {t('in v2ray format. Kova checks that the files can be read, but can’t verify who published them.')}{/if}
        </p>
        {#if customOpen}
          <div class="form gap">
            <div class="fld full"><label for="geo-ip">{t('geoip.dat address')}</label><input id="geo-ip" class="input mono small" bind:value={draft.geoip} placeholder="https://…/geoip.dat" /></div>
            <div class="fld full"><label for="geo-site">{t('geosite.dat address')}</label><input id="geo-site" class="input mono small" bind:value={draft.geosite} placeholder="https://…/geosite.dat" /></div>
          </div>
          <div class="actions gap">
            <button class="btn primary" class:busy={geoBusy} disabled={geoBusy || !draft.geoip.trim() || !draft.geosite.trim()}
              onclick={() => setGeo({ kind: 'custom', geoip: draft.geoip.trim(), geosite: draft.geosite.trim() }, t('Custom routing data downloaded'))}>{#if geoBusy}<Icon name="refresh" />{t('Downloading…')}{:else}{t('Download and use')}{/if}</button>
            <button class="btn ghost" onclick={() => (customOpen = false)}>{t('Cancel')}</button>
          </div>
        {/if}
        {#if geo}
          <dl class="kv gap">
            <dt>{t('Installed')}</dt>
            <dd>{geo.installed ? `${geoSourceName(geo.installed.source)} · ${fmtAgo(geo.installed.updated_at)}` : geo.files.length ? t('Bundled with Xray') : t('Not downloaded yet')}</dd>
            {#if geo.installed}<dt>{t('Checksum')}</dt><dd>{geo.installed.verified ? t('Verified against the publisher') : t('Not available for this source')}</dd>{/if}
            {#each geo.files as f (f.name)}<dt class="mono">{f.name}</dt><dd>{fmtBytes(f.size)} · {tn(f.categories, '{n} category', '{n} categories')}</dd>{/each}
          </dl>
          {#if geo.missing.length}
            <p class="hint warn-text">{t('Missing for your presets: {list}. Pick a source that has them or turn the preset off.', { list: geo.missing.join(', ') })}</p>
          {/if}
        {/if}
        {#if !customOpen}
          <div class="actions gap"><button class="btn" class:busy={geoBusy} disabled={geoBusy} onclick={updateGeo}><Icon name="refresh" />{geoBusy ? t('Downloading…') : t('Update now')}</button></div>
        {/if}
      </FoldCard>
      <FoldCard id="cores" title={t('Cores')}>
        {#snippet aside()}{#if !app.mobile}<button class="btn ghost aside-btn" class:busy={checking} onclick={() => loadCores(true)} disabled={checking}><Icon name="refresh" />{t('Check for updates')}</button>{/if}{/snippet}
        {#if !cores.length}
          <!-- Same height as the rows to come, so nothing below moves when they do. -->
          {#each ['mihomo', 'xray'] as k (k)}
            <div class="core"><span class="ico">{CORE_NAMES[k as CoreKind][0]}</span><span class="t">{CORE_NAMES[k as CoreKind]}</span><span class="chip">…</span><span class="d">&nbsp;</span></div>
          {/each}
        {/if}
        {#each cores as c (c.core)}
          {@const name = CORE_NAMES[c.core]}
          {@const update = c.latest && norm(c.latest) !== norm(c.installed)}
          <div class="core">
            <span class="ico">{name[0]}</span>
            <span class="t">{name} {c.installed ?? ''}</span>
            {#if !app.mobile && (!c.installed || update)}
              <button class="btn primary" class:busy={installing === c.core} disabled={installing !== null} onclick={() => install(c.core)}>
                {#if installing === c.core}<Icon name="refresh" />{t('Installing…')}{:else}{c.installed ? t('Update to {version}', { version: c.latest ?? '' }) : t('Install')}{/if}</button>
            {:else}
              <span class="chip">{c.latest ? t('Up to date') : t('Installed')}</span>
            {/if}
            <span class="d">{c.core === 'xray' ? t('XTLS/Xray-core · XHTTP, Reality, bridged nodes') : c.core === 'shoes' ? t('cfal/shoes · NaiveProxy servers; installed when a profile needs it') : t('MetaCubeX/mihomo · rules, groups, TUN, most protocols')}{!c.installed ? ` · ${t('not installed')}` : ''}</span>
          </div>
        {/each}
        <p class="hint">{app.mobile ? t('Included with the app and updated together with it.') : t('Downloaded from the official GitHub releases. Kova checks each file’s SHA-256 before installing it.')}</p>
      </FoldCard>

      <FoldCard id="app" title={t('App')}>
        <div class="rows">
          {#if !app.mobile}
            <div class="row-sw"><div><div class="t">{t('Launch at sign-in')}</div></div>
              <button class="switch" role="switch" aria-label={t('Launch at sign-in')} aria-checked={s.launch_at_login} onclick={setAutostart}></button></div>
            <div class="row-sw"><div><div class="t">{t('Start in the tray')}</div></div>
              <button class="switch" role="switch" aria-label={t('Start in the tray')} aria-checked={s.start_minimized} onclick={() => toggle((x) => x.start_minimized, (x, v) => (x.start_minimized = v))}></button></div>
          {/if}
          <div class="row-sw"><div><div class="t">{t('Connect on launch')}</div></div>
            <button class="switch" role="switch" aria-label={t('Connect on launch')} aria-checked={s.connect_on_launch} onclick={() => toggle((x) => x.connect_on_launch, (x, v) => (x.connect_on_launch = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Backup')}</div><div class="d">{t('All profiles and settings in one file, for another device or for safekeeping.')}</div></div>
            <div class="actions"><button class="btn" onclick={() => (backup = 'save')}>{t('Save…')}</button><button class="btn" onclick={() => (backup = 'restore')}>{t('Restore…')}</button></div></div>
        </div>
        {#if backup}<BackupModal mode={backup} onclose={() => (backup = null)} />{/if}
        <div class="form gap">
          <div class="fld full"><span class="lbl">{t('Theme')}</span>
            <ThemePicker /></div>
          <div class="fld full"><span class="lbl">{t('Layout')}</span>
            <div class="layout-row">
              <div class="seg" role="group" aria-label={t('Layout preset')}>
                {#each PRESETS as p (p)}<button aria-pressed={layoutPreset === p} onclick={() => app.applyPreset(p)}>{PRESET_NAME[p]}</button>{/each}
              </div>
              <button class="btn" onclick={() => { app.editingLayout = true; app.go('dashboard'); }}><Icon name="edit" />{t('Edit layout')}</button>
            </div>
            <small class="muted">{layoutPreset ? '' : t('Your own layout.') + ' '}{app.form === 'phone' ? t('Phones and computers each keep their own layout; this one is for the phone.') : t('Phones and computers each keep their own layout; this one is for this computer.')} {t('Hidden tabs still open from links to them, and Settings always stays.')}</small></div>
          <div class="fld full"><span class="lbl">{t('Language')}</span>
            <!-- Language names stay in their own language, so they can be found whatever is shown. -->
            <div class="seg" role="group" aria-label={t('Language')}>
              {#each [['system', t('Match system')], ['en', 'English'], ['ru', 'Русский'], ['fa', 'فارسی'], ['zh', '中文']] as [v, l] (v)}
                <button aria-pressed={s.language === v} onclick={() => set((x) => (x.language = v))}>{l}</button>
              {/each}
            </div></div>
          <div class="fld"><label for="st-log">{t('Core log level')}</label>
            <select id="st-log" class="input" value={s.core.log_level} onchange={(e) => set((x) => (x.core.log_level = e.currentTarget.value as Settings['core']['log_level']))}>
              <option value="debug">{t('Debug')}</option><option value="info">{t('Info')}</option><option value="warning">{t('Warning')}</option><option value="error">{t('Error')}</option><option value="silent">{t('Silent')}</option>
            </select></div>
        </div>
        {#if !app.mobile}
          <div class="row-sw data"><div><div class="t">{t('Data folder')}</div><div class="d mono selectable">{app.dataDir}</div></div>
            <button class="btn" onclick={() => app.run(() => openPath(app.dataDir))}><Icon name="folder" />{t('Open')}</button></div>
          <div class="row-sw"><div><div class="t">{t('Quit Kova')}</div><div class="d">{t('Disconnects and restores your system proxy')}</div></div>
            <button class="btn danger" onclick={() => api.quit()}>{t('Quit')}</button></div>
        {/if}
      </FoldCard>

      <FoldCard id="look" title={t('Look')}>
        {#snippet aside()}
          <div class="look-tools">
            <button class="btn ghost aside-btn" onclick={() => (themeFile = 'import')}><Icon name="file" />{t('Import')}</button>
            <button class="btn ghost aside-btn" onclick={() => (themeFile = 'export')}><Icon name="down" />{t('Export')}</button>
            {#if !isEmpty(look)}<button class="btn ghost aside-btn" onclick={() => app.saveSettings((x) => (x.look = null), t('Look reset'))}><Icon name="refresh" />{t('Reset look')}</button>{/if}
          </div>
        {/snippet}
        {#if themeFile}<ThemeFileModal mode={themeFile} onclose={() => (themeFile = null)} />{/if}
        <p class="hint">{t('Drawn over the theme above: pick only what you want to change. Changes show at once.')}</p>
        <div class="form gap">
          <div class="fld full"><span class="lbl">{t('Colours')}</span>
            <div class="swatches">
              {#each COLOR_KEYS as k (k)}
                <div class="sw">
                  <label class="sw-pick" style="--c:{swatch(k)}">
                    <input type="color" value={swatch(k)} oninput={(e) => setColor(k, e.currentTarget.value, false)} onchange={(e) => setColor(k, e.currentTarget.value, true)} aria-label={COLOR_NAME[k]} />
                  </label>
                  <span class="sw-t">{COLOR_NAME[k]}<small>{look.colors?.[k] ? look.colors[k] : t('from the theme')}</small></span>
                  {#if look.colors?.[k]}<button class="btn ghost sw-x" onclick={() => setColor(k, undefined, true)} aria-label={t('Use the theme colour')} title={t('Use the theme colour')}><Icon name="x" /></button>{/if}
                </div>
              {/each}
            </div>
            {#if app.lookAdjusted}<small class="warn-t">{t('The text colour was hard to read on these cards, so Kova uses black or white text instead.')}</small>{/if}
          </div>
          <div class="fld"><label for="lk-font">{t('Font')}</label>
            <select id="lk-font" class="input" value={look.font ?? 'onest'} onchange={(e) => app.saveLook((l) => { const f = e.currentTarget.value as FontId; if (f === 'onest') delete l.font; else l.font = f; })}>
              {#each Object.keys(FONTS) as f (f)}<option value={f}>{FONT_NAME[f as FontId]}</option>{/each}
            </select></div>
          <div class="fld"><span class="lbl">{t('Density')}</span>
            <div class="seg" role="group" aria-label={t('Density')}>
              <button aria-pressed={look.density !== 'compact'} onclick={() => app.saveLook((l) => delete l.density)}>{t('Comfortable')}</button>
              <button aria-pressed={look.density === 'compact'} onclick={() => app.saveLook((l) => (l.density = 'compact'))}>{t('Compact')}</button>
            </div></div>
          <div class="fld full"><label for="lk-scale">{t('Size')} <span class="num val">{look.scale ?? 100}%</span></label>
            <input id="lk-scale" type="range" min={LIMITS.scale[0]} max={LIMITS.scale[1]} step="5" value={look.scale ?? 100}
              oninput={(e) => app.previewLook((l) => (l.scale = Number(e.currentTarget.value)))} onchange={(e) => app.saveLook((l) => { const v = Number(e.currentTarget.value); if (v === 100) delete l.scale; else l.scale = v; })} /></div>
          <div class="fld full"><label for="lk-radius">{t('Corners')} <span class="num val">{look.radius ?? DEFAULT_RADIUS}px</span></label>
            <input id="lk-radius" type="range" min={LIMITS.radius[0]} max={LIMITS.radius[1]} step="1" value={look.radius ?? DEFAULT_RADIUS}
              oninput={(e) => app.previewLook((l) => (l.radius = Number(e.currentTarget.value)))} onchange={(e) => app.saveLook((l) => { const v = Number(e.currentTarget.value); if (v === DEFAULT_RADIUS) delete l.radius; else l.radius = v; })} /></div>
          <div class="fld full"><label for="lk-glass">{t('Card transparency')} <span class="num val">{look.glass ?? 0}%</span></label>
            <input id="lk-glass" type="range" min={LIMITS.glass[0]} max={LIMITS.glass[1]} step="5" value={look.glass ?? 0}
              oninput={(e) => app.previewLook((l) => (l.glass = Number(e.currentTarget.value)))} onchange={(e) => app.saveLook((l) => { const v = Number(e.currentTarget.value); if (!v) delete l.glass; else l.glass = v; })} />
            <small class="muted">{t('Shows what is behind the cards, blurred.')}</small></div>
        </div>

        <div class="pics">
          <div class="fld full"><label for="lk-slot">{t('Pictures')}</label>
            <select id="lk-slot" class="input" bind:value={slot}>
              {#each SLOTS as [id, name] (id)}<option value={id}>{name}{look.pictures?.[id] ? ' ✓' : ''}</option>{/each}
            </select></div>
          <div class="pic-row">
            <div class="pic-prev" style="aspect-ratio: {size[0]} / {size[1]}; {pic && picUrl ? pictureStyle(pic, picUrl) : ''}">{#if !pic}<span>{t('No picture')}</span>{/if}</div>
            <div class="pic-side">
              <span class="d">{t('Best at {w} × {h}. PNG, JPG, WebP or animated GIF, up to 10 MB; Kova keeps its own copy.', { w: size[0], h: size[1] })}</span>
              <div class="actions">
                <input type="file" accept="image/png,image/jpeg,image/webp,image/gif" hidden bind:this={picInput} onchange={(e) => choosePicture(e.currentTarget.files?.[0])} />
                <button class="btn primary" class:busy={picking} disabled={picking} onclick={() => picInput?.click()}><Icon name="plus" />{pic ? t('Change picture') : t('Choose picture')}</button>
                {#if !app.mobile}<button class="btn" onclick={saveTemplate}><Icon name="down" />{t('Save template')}</button>{/if}
                {#if pic}<button class="btn ghost danger" onclick={dropPicture}><Icon name="trash" />{t('Remove')}</button>{/if}
              </div>
            </div>
          </div>
          {#if pic}
            <div class="form gap">
              <div class="fld"><span class="lbl">{t('Fit')}</span>
                <div class="seg" role="group" aria-label={t('Fit')}>
                  {#each [['cover', t('Fill')], ['contain', t('Whole picture')], ['tile', t('Tile')]] as [v, l] (v)}
                    <button aria-pressed={(pic.fit ?? 'cover') === v} onclick={() => setPic((p) => (p.fit = v as Fit))}>{l}</button>
                  {/each}
                </div></div>
              <div class="fld"><label for="lk-pos">{t('Position')}</label>
                <select id="lk-pos" class="input" value={pic.pos ?? 'center'} onchange={(e) => setPic((p) => (p.pos = e.currentTarget.value as Pos))}>
                  <option value="center">{t('Centre')}</option><option value="top">{t('Top')}</option><option value="bottom">{t('Bottom')}</option><option value="left">{t('Left')}</option><option value="right">{t('Right')}</option>
                </select></div>
              <div class="fld full"><label for="lk-dim">{t('Dim')} <span class="num val">{pic.dim ?? DEFAULT_DIM}%</span></label>
                <input id="lk-dim" type="range" min="0" max="90" step="5" value={pic.dim ?? DEFAULT_DIM}
                  oninput={(e) => setPic((p) => (p.dim = Number(e.currentTarget.value)), false)} onchange={(e) => setPic((p) => (p.dim = Number(e.currentTarget.value)))} />
                <small class="muted">{t('Covers the picture with the window colour, so text stays readable.')}</small></div>
              {#if isBackdrop(slot)}
                <div class="fld full"><label for="lk-blur">{t('Blur')} <span class="num val">{pic.blur ?? 0}px</span></label>
                  <input id="lk-blur" type="range" min="0" max="20" step="1" value={pic.blur ?? 0}
                    oninput={(e) => setPic((p) => (p.blur = Number(e.currentTarget.value)), false)} onchange={(e) => setPic((p) => (p.blur = Number(e.currentTarget.value)))} /></div>
              {/if}
            </div>
          {/if}
        </div>
        <details class="adv" bind:open={cssOpen}>
          <summary>{t('Custom CSS')} <span class="muted">· {t('advanced')}</span></summary>
          <p class="hint">{t('Any CSS, applied last so it wins. After Apply, a bar asks to keep it and undoes it after 15 seconds otherwise, so a broken style can’t lock the window. On a computer, Ctrl+Shift+K also turns it off until Kova restarts.')}</p>
          <textarea class="input mono css" rows="8" spellcheck="false" bind:value={cssDraft} placeholder=".card {'{'} border-radius: 18px; {'}'}"></textarea>
          <div class="actions">
            <button class="btn primary" onclick={applyCss} disabled={cssDraft === (look.css ?? '')}>{t('Apply')}</button>
            {#if look.css}<button class="btn ghost danger" onclick={() => { cssDraft = ''; app.tryCss(''); }}>{t('Remove')}</button>{/if}
            {#if app.cssOff}<span class="chip warn">{t('Off until Kova restarts')}</span>{/if}
          </div>
        </details>
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Card outlines')}</div></div>
            <button class="switch" role="switch" aria-label={t('Card outlines')} aria-checked={look.borders !== false} onclick={() => app.saveLook((l) => { if (l.borders === false) delete l.borders; else l.borders = false; })}></button></div>
        </div>
      </FoldCard>


      <FoldCard id="updates" title={t('Updates')}>
        {#snippet aside()}<span class="aside">Kova {appVersion}</span>{/snippet}
        {#if app.update}
          <div class="upd">
            <div class="t">{t('Kova {version} is available', { version: app.update.version })}</div>
            {#if app.update.date}<div class="d">{t('Released {date}', { date: app.update.date.slice(0, 10) })}</div>{/if}
            {#if app.update.body}<p class="notes">{@html notesHtml(app.update.body)}</p>{/if}
            {#if app.updateProgress !== null}
              <div class="bar"><i style="width:{Math.round(app.updateProgress * 100)}%"></i></div>
              <div class="d">{app.updateProgress < 1 ? t('Downloading… {pct}%', { pct: Math.round(app.updateProgress * 100) }) : app.mobile ? t('Opening Android’s installer') : t('Installing, Kova will restart')}</div>
            {:else if app.mobile}
              <div class="actions gap"><button class="btn primary" onclick={() => app.installUpdate()}><Icon name="down" />{t('Download update')}</button></div>
              <p class="hint">{t('Kova downloads and checks the update, then Android asks you to confirm the install. Your profiles and settings stay.')}</p>
            {:else}
              <div class="actions gap"><button class="btn primary" onclick={() => app.installUpdate()}><Icon name="down" />{t('Install and restart')}</button></div>
              <p class="hint">{t('Kova disconnects first. Windows asks for administrator approval because Kova is installed for all users.')}</p>
            {/if}
          </div>
        {/if}
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Check for updates on launch')}</div><div class="d">{t('Releases are signed; Kova only installs updates signed with its key')}</div></div>
            <button class="switch" role="switch" aria-label={t('Check for updates on launch')} aria-checked={s.check_updates} onclick={() => toggle((x) => x.check_updates, (x, v) => (x.check_updates = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Get beta versions')}</div><div class="d">{t('New features earlier, before they are fully tested. Kova updates to whichever is newer, beta or stable.')}</div></div>
            <button class="switch" role="switch" aria-label={t('Get beta versions')} aria-checked={s.beta_updates}
              onclick={async () => { await set((x) => (x.beta_updates = !x.beta_updates)); void app.checkForUpdate(true); }}></button></div>
        </div>
        <div class="actions gap"><button class="btn" class:busy={app.checkingUpdate} disabled={app.checkingUpdate || app.updateProgress !== null} onclick={() => app.checkForUpdate()}><Icon name="refresh" />{t('Check now')}</button></div>
      </FoldCard>

      <FoldCard id="contacts" title={t('What Kova connects to')} closed>
        <div class="contacts">
          <p class="hint">{t('Besides your servers, Kova contacts only these, and the last four only when you use that feature:')}</p>
          <dl>
            <dt class="mono">github.com</dt><dd>{t('Kova updates, cores and routing data')}</dd>
            <dt>{t('Your subscription’s address')}</dt><dd>{t('Updating the profile. Kova sends a random device ID made for this install, which providers use to count devices')}</dd>
            <dt class="mono">www.gstatic.com</dt><dd>{t('Latency tests and health checks, through your servers')}</dd>
            <dt class="mono">ipwho.is, ipinfo.io</dt><dd>{t('The Exit point card')}</dd>
            <dt class="mono">ya.ru</dt><dd>{t('What’s wrong? (whether the network works)')}</dd>
            <dt class="mono">api.ipify.org</dt><dd>{t('Privacy check')}</dd>
            <dt class="mono">speed.cloudflare.com</dt><dd>{t('Speed test')}</dd>
            <dt>{t('Sites in Site check')}</dt><dd>{t('Site check')}</dd>
          </dl>
        </div>
      </FoldCard>

      <FoldCard id="donate" anchor="donate" title={t('Support Kova')}>
        {#snippet aside()}<span class="aside">{t('by {author}', { author: 'scarletjess77' })}</span>{/snippet}
        <Donate />
      </FoldCard>
  </div>
{/if}

<style>
  .contacts dl { display: grid; grid-template-columns: minmax(0, auto) minmax(0, 1fr); gap: 6px 14px; margin: 8px 0 0; font-size: 12px; }
  .contacts dt { overflow-wrap: anywhere; }
  .contacts dd { margin: 0; color: var(--muted); }
  .set { columns: 2; column-gap: 14px; }
  .set > :global(.card) { break-inside: avoid; margin-bottom: 14px; }
  @container (max-width: 900px) { .set { columns: 1; } }
  .aside-btn, .aside-sw { margin-inline-start: auto; }
  .core { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 2px 12px; align-items: center; padding: 12px 0; border-top: 1px solid var(--line); }
  .core:first-of-type { border-top: 0; padding-top: 0; }
  .ico { grid-row: span 2; width: 36px; height: 36px; border-radius: 8px; display: grid; place-items: center; background: var(--sunk); border: 1px solid var(--line); font: 500 12px var(--f-display); }
  .core .t { font-weight: 500; }
  .core .d { font-size: 12px; color: var(--muted); }
  .core .btn, .core .chip { grid-row: span 2; }
  .rows { margin-top: 8px; }
  .small { font-size: 12px; }
  .first { border-top: 0; padding-top: 0; margin-bottom: 8px; }
  .gap { margin-top: 10px; }
  .data { margin-top: 12px; }
  .data .d { word-break: break-all; }
  .warn-text { color: var(--warn); }
  .upd { display: flex; flex-direction: column; gap: 6px; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--win); margin-bottom: 4px; }
  .upd .t { font-weight: 500; }
  .upd .d { font-size: 12px; color: var(--muted); }
  .notes { margin: 4px 0 0; font-size: 12px; color: var(--muted); white-space: pre-wrap; }
  .svc { display: flex; align-items: center; gap: 12px; padding: 10px 12px; margin: 8px 0 4px; border: 1px solid var(--line); border-radius: 8px; background: var(--win); flex-wrap: wrap; }
  .svc-text { flex: 1; min-width: 200px; }
  .svc .t { font-weight: 500; }
  .svc .d { font-size: 12px; color: var(--muted); }
  .led { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; }
  .led.on { background: var(--ok); box-shadow: 0 0 0 3px var(--ok-soft); }
  .mobile-server { max-width: 46%; }
  .rows > .t { font-weight: 500; }
  .rows > .d { font-size: 12px; color: var(--muted); margin-top: 2px; }
  .countries { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .countries .chip { cursor: pointer; color: var(--fg); }
  .countries .chip.off { color: var(--faint); text-decoration: line-through; background: transparent; }
  .layout-row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .swatches { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 8px; }
  .sw { display: flex; align-items: center; gap: 10px; padding: 8px; border: 1px solid var(--line); border-radius: var(--r); background: var(--sunk); min-width: 0; }
  .sw-pick { position: relative; flex: none; width: 32px; height: 32px; border-radius: 8px; background: var(--c); box-shadow: inset 0 0 0 1px var(--line-2); cursor: pointer; overflow: hidden; }
  .sw-pick input { position: absolute; inset: 0; opacity: 0; cursor: pointer; width: 100%; height: 100%; }
  .sw-t { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 13px; font-weight: 500; }
  .sw-t small { font: 400 11px var(--f-mono); color: var(--faint); }
  .sw-x { padding: 4px; }
  .val { color: var(--muted); font-weight: 400; margin-inline-start: 6px; }
  .warn-t { color: var(--warn); }
  input[type='range'] { width: 100%; accent-color: var(--accent); }
  .pics { display: flex; flex-direction: column; gap: 12px; margin-top: 16px; padding-top: 16px; border-top: 1px solid var(--line); }
  .pic-row { display: grid; grid-template-columns: minmax(0, 220px) minmax(0, 1fr); gap: 16px; align-items: start; }
  @container (max-width: 560px) { .pic-row { grid-template-columns: minmax(0, 1fr); } }
  .pic-prev { width: 100%; max-height: 220px; border-radius: var(--r); border: 1px dashed var(--line-2); background-color: var(--sunk); display: grid; place-items: center; color: var(--faint); font-size: 12px; overflow: hidden; }
  .pic-side { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .pic-side .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .adv { margin-top: 16px; padding-top: 12px; border-top: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; }
  .adv summary { cursor: pointer; font-weight: 500; font-size: 13px; }
  .adv .css { width: 100%; min-height: 140px; resize: vertical; font-size: 12px; line-height: 1.5; user-select: text; }
  .adv .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .look-tools { display: flex; gap: 4px; flex-wrap: wrap; margin-inline-start: auto; }
</style>
