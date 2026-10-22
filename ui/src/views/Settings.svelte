<script lang="ts">
  import { disable as autostartOff, enable as autostartOn, isEnabled as autostartIsOn } from '@tauri-apps/plugin-autostart';
  import { getVersion } from '@tauri-apps/api/app';
  import { openPath } from '@tauri-apps/plugin-opener';
  import { api, errText, type CoreKind, type CoreStatus, type GeoInfo, type GeoSource, type ServiceView, type Settings } from '../lib/api';
  import { fmtAgo, fmtBytes } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import AppPicker from '../components/AppPicker.svelte';
  import Donate from '../components/Donate.svelte';
  import Icon from '../components/Icon.svelte';

  let appVersion = $state('');
  let pickingApps = $state(false);
  let lanIp = $state<string | null>(null);
  $effect(() => { if (app.settings?.core.allow_lan) void api.lanAddress().then((v) => (lanIp = v), () => {}); });
  getVersion().then((v) => (appVersion = v), () => {});

  let s = $derived(app.settings);
  let cores = $state<CoreStatus[]>([]);
  let checking = $state(false);
  let installing = $state<CoreKind | null>(null);

  async function loadCores(latest: boolean) {
    checking = latest;
    cores = (await app.run(() => api.coreStatus(latest))) ?? cores;
    checking = false;
  }
  $effect(() => { void app.coresVersion; void loadCores(true); });

  const norm = (v: string | null) => (v ?? '').replace(/^v/, '');
  async function install(c: CoreKind) {
    installing = c;
    await app.run(() => api.installCore(c), (tag) => t('{core} {tag} installed', { core: c === 'xray' ? 'Xray' : 'Mihomo', tag }));
    installing = null;
    await loadCores(false);
  }

  const set = (f: (x: Settings) => void, ok?: string) => app.saveSettings(f, ok);
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
    await app.run(() => api.serviceInstall(), svc?.installed ? t('Kova service updated') : t('Kova service installed. TUN mode no longer needs administrator rights'));
    svcBusy = null;
    await loadSvc();
  }
  async function removeSvc() {
    svcBusy = 'remove';
    await app.run(() => api.serviceUninstall(), t('Kova service removed'));
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
  <div class="set">
    <!-- Connection settings on the left, the app itself on the right. Two independent
         columns so a tall card never leaves a gap beside a short one. -->
    <div class="col">
      <section class="card">
        <div class="card-h"><h2>{t('Network')}</h2></div>
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
              {#if lanIp}<div class="d">{t('Address for other devices')} <span class="mono selectable">{lanIp}:{s.core.mixed_port}</span> {t('(HTTP and SOCKS5)')}</div>{/if}
              <div class="d">{t('Other devices sign in as {user} with the password', { user: 'kova' })} <span class="mono selectable">{s.core.local_password}</span></div>
            {/if}</div>
            <button class="switch" role="switch" aria-label={t('Allow connections from LAN')} aria-checked={s.core.allow_lan} onclick={() => toggle((x) => x.core.allow_lan, (x, v) => (x.core.allow_lan = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Switch away from a dead node')}</div><div class="d">{t('When the chosen node stops answering, move to the fastest one that works and say so')}</div></div>
            <button class="switch" role="switch" aria-label={t('Switch away from a dead node')} aria-checked={s.auto_switch} onclick={() => toggle((x) => x.auto_switch, (x, v) => (x.auto_switch = v))}></button></div>
          <div class="row-sw"><div><div class="t">IPv6</div></div>
            <button class="switch" role="switch" aria-label="IPv6" aria-checked={s.core.ipv6} onclick={() => toggle((x) => x.core.ipv6, (x, v) => (x.core.ipv6 = v))}></button></div>
          <div class="row-sw"><div><div class="t">{t('Sniff domains')}</div><div class="d">{t('Reads TLS and HTTP hostnames so domain rules also work for raw IP connections')}</div></div>
            <button class="switch" role="switch" aria-label={t('Sniff domains')} aria-checked={s.core.sniff} onclick={() => toggle((x) => x.core.sniff, (x, v) => (x.core.sniff = v))}></button></div>
        </div>
      </section>

      {#if app.mobile}
      <section class="card">
        <div class="card-h"><h2>VPN</h2></div>
        <div class="rows">
          <div class="row-sw"><div><div class="t">{t('Apps through the VPN')}</div>
            <div class="d">{s.vpn_apps.mode === 'all' ? t('All apps') : s.vpn_apps.mode === 'only'
              ? tn(s.vpn_apps.apps.length, 'Only {n} chosen app', 'Only {n} chosen apps')
              : tn(s.vpn_apps.apps.length, 'All but {n} chosen app', 'All but {n} chosen apps')}</div></div>
            <button class="btn" onclick={() => (pickingApps = true)}>{t('Choose')}</button></div>
          <div class="row-sw"><div><div class="t">{t('Always-on VPN')}</div><div class="d">{t('Android starts Kova’s VPN by itself, after a restart too. Pick Kova in Android’s VPN settings and turn on Always-on VPN.')}</div></div>
            <button class="btn" onclick={() => app.run(() => api.openVpnSettings())}>{t('Open')}</button></div>
          <div class="row-sw"><div><div class="t">{t('Quick Settings tile')}</div><div class="d">{t('Swipe down twice, tap the pencil and drag the Kova tile in to connect and disconnect from there.')}</div></div></div>
        </div>
      </section>
      {/if}
      {#if pickingApps}<AppPicker onclose={() => (pickingApps = false)} />{/if}

      {#if !app.mobile}
      <section class="card">
        <div class="card-h"><h2>{t('TUN mode')}</h2>
          <button class="switch aside-sw" role="switch" aria-label={t('TUN mode')} aria-checked={s.core.tun.enabled} onclick={() => toggle((x) => x.core.tun.enabled, (x, v) => (x.core.tun.enabled = v))}></button></div>
        <div class="fld"><span class="lbl">{t('Network stack')}</span>
          <div class="seg" role="group" aria-label={t('Network stack')}>
            {#each [['mixed', t('Mixed')], ['system', t('System')], ['gvisor', 'gVisor']] as [v, l] (v)}
              <button aria-pressed={s.core.tun.stack === v} onclick={() => set((x) => (x.core.tun.stack = v as Settings['core']['tun']['stack']))}>{l}</button>
            {/each}
          </div>
        </div>
        {#if app.platform === 'linux' || app.platform === 'macos'}
          <p class="hint">{t('TUN captures every app, including games and UDP. On Linux and macOS it needs administrator rights for now: start Kova with sudo to use it. The system proxy works without them.')}</p>
        {:else}
          <p class="hint">{t('TUN captures every app, including games and UDP. It needs system rights, which the Kova service provides so Kova itself never runs as administrator.')}</p>
        {/if}
        {#if svc?.supported}
          <div class="svc">
            <span class="led" class:on={svc.running}></span>
            <div class="svc-text">
              <div class="t">{t('Kova service')}</div>
              <div class="d">
                {#if svc.running}{t('Running · version {version}', { version: svc.version ?? '' })}{svc.core_version ? ` · Mihomo ${svc.core_version}` : ''}{svcOutdated ? ` · ${t('from another Kova build, update it')}` : ''}
                {:else if svc.installed}{t('Installed but not running')}
                {:else}{t('Not installed. Installing asks for administrator approval once.')}{/if}
              </div>
            </div>
            <div class="actions">
              {#if !svc.installed || !svc.running || svcOutdated}
                <button class="btn primary" class:busy={svcBusy === 'install'} disabled={svcBusy !== null} onclick={installSvc}>
                  {#if svcBusy === 'install'}<Icon name="refresh" />{t('Waiting for approval…')}{:else}{svc.installed ? t('Repair or update') : t('Install')}{/if}</button>
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
      </section>
      {/if}

      <section class="card">
        <div class="card-h"><h2>DNS</h2></div>
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
      </section>

      <section class="card">
        <div class="card-h"><h2>{t('Routing data')}</h2><span class="aside">{t('GeoIP and GeoSite lists used by rules')}</span></div>
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
      </section>
    </div>
    <div class="col">
      <section class="card">
        <div class="card-h"><h2>{t('Cores')}</h2>
          {#if !app.mobile}<button class="btn ghost aside-btn" class:busy={checking} onclick={() => loadCores(true)} disabled={checking}><Icon name="refresh" />{t('Check for updates')}</button>{/if}</div>
        {#each cores as c (c.core)}
          {@const name = c.core === 'xray' ? 'Xray' : 'Mihomo'}
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
            <span class="d">{c.core === 'xray' ? t('XTLS/Xray-core · XHTTP, Reality, bridged nodes') : t('MetaCubeX/mihomo · rules, groups, TUN, most protocols')}{!c.installed ? ` · ${t('not installed')}` : ''}</span>
          </div>
        {/each}
        <p class="hint">{app.mobile ? t('Included with the app and updated together with it.') : t('Downloaded from the official GitHub releases. Kova checks each file’s SHA-256 before installing it.')}</p>
      </section>

      <section class="card">
        <div class="card-h"><h2>{t('App')}</h2></div>
        <div class="rows">
          {#if !app.mobile}
            <div class="row-sw"><div><div class="t">{t('Launch at sign-in')}</div></div>
              <button class="switch" role="switch" aria-label={t('Launch at sign-in')} aria-checked={s.launch_at_login} onclick={setAutostart}></button></div>
            <div class="row-sw"><div><div class="t">{t('Start in the tray')}</div></div>
              <button class="switch" role="switch" aria-label={t('Start in the tray')} aria-checked={s.start_minimized} onclick={() => toggle((x) => x.start_minimized, (x, v) => (x.start_minimized = v))}></button></div>
          {/if}
          <div class="row-sw"><div><div class="t">{t('Connect on launch')}</div></div>
            <button class="switch" role="switch" aria-label={t('Connect on launch')} aria-checked={s.connect_on_launch} onclick={() => toggle((x) => x.connect_on_launch, (x, v) => (x.connect_on_launch = v))}></button></div>
        </div>
        <div class="form gap">
          <div class="fld"><span class="lbl">{t('Theme')}</span>
            <div class="seg" role="group" aria-label={t('Theme')}>
              {#each [['dark', t('Dark')], ['light', t('Light')], ['system', t('Match system')]] as [v, l] (v)}
                <button aria-pressed={s.theme === v} onclick={() => set((x) => (x.theme = v as Settings['theme']))}>{l}</button>
              {/each}
            </div></div>
          <div class="fld"><span class="lbl">{t('Language')}</span>
            <!-- Language names stay in their own language, so they can be found whatever is shown. -->
            <div class="seg" role="group" aria-label={t('Language')}>
              {#each [['system', t('Match system')], ['en', 'English'], ['ru', 'Русский']] as [v, l] (v)}
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
      </section>

      <section class="card">
        <div class="card-h"><h2>{t('Updates')}</h2><span class="aside">Kova {appVersion}</span></div>
        {#if app.update}
          <div class="upd">
            <div class="t">{t('Kova {version} is available', { version: app.update.version })}</div>
            {#if app.update.date}<div class="d">{t('Released {date}', { date: app.update.date.slice(0, 10) })}</div>{/if}
            {#if app.update.body}<p class="notes">{app.update.body.slice(0, 400)}</p>{/if}
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
      </section>

      <section class="card">
        <details class="contacts">
          <summary><h2>{t('What Kova connects to')}</h2></summary>
          <p class="hint">{t('Besides your servers, Kova contacts only these, and the last four only when you use that feature:')}</p>
          <dl>
            <dt class="mono">github.com</dt><dd>{t('Kova updates, cores and routing data')}</dd>
            <dt>{t('Your subscription’s address')}</dt><dd>{t('Updating the profile')}</dd>
            <dt class="mono">www.gstatic.com</dt><dd>{t('Latency tests and health checks, through your servers')}</dd>
            <dt class="mono">ipwho.is, ipinfo.io</dt><dd>{t('The Exit point card')}</dd>
            <dt class="mono">ya.ru</dt><dd>{t('What’s wrong? (whether the network works)')}</dd>
            <dt class="mono">api.ipify.org</dt><dd>{t('Privacy check')}</dd>
            <dt class="mono">speed.cloudflare.com</dt><dd>{t('Speed test')}</dd>
            <dt>{t('Sites in Site check')}</dt><dd>{t('Site check')}</dd>
          </dl>
        </details>
      </section>

      <section class="card" id="donate">
        <div class="card-h"><h2>{t('Support Kova')}</h2><span class="aside">{t('by {author}', { author: 'scarletjess77' })}</span></div>
        <Donate />
      </section>
    </div>
  </div>
{/if}

<style>
  .contacts summary { cursor: pointer; list-style: none; }
  .contacts summary h2 { display: inline; font: 600 13.5px var(--f-ui); }
  .contacts dl { display: grid; grid-template-columns: minmax(0, auto) minmax(0, 1fr); gap: 6px 14px; margin: 8px 0 0; font-size: 12.5px; }
  .contacts dt { overflow-wrap: anywhere; }
  .contacts dd { margin: 0; color: var(--muted); }
  .set { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; align-items: start; }
  .col { display: flex; flex-direction: column; gap: 14px; min-width: 0; }
  @container (max-width: 900px) { .set { grid-template-columns: minmax(0, 1fr); } }
  .aside-btn, .aside-sw { margin-left: auto; }
  .core { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 2px 12px; align-items: center; padding: 12px 0; border-top: 1px solid var(--line); }
  .core:first-of-type { border-top: 0; padding-top: 0; }
  .ico { grid-row: span 2; width: 36px; height: 36px; border-radius: 8px; display: grid; place-items: center; background: var(--sunk); border: 1px solid var(--line); font: 700 12px var(--f-display); }
  .core .t { font-weight: 600; }
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
  .upd .t { font-weight: 600; }
  .upd .d { font-size: 12px; color: var(--muted); }
  .notes { margin: 4px 0 0; font-size: 12.5px; color: var(--muted); white-space: pre-wrap; }
  .svc { display: flex; align-items: center; gap: 12px; padding: 10px 12px; margin: 8px 0 4px; border: 1px solid var(--line); border-radius: 8px; background: var(--win); flex-wrap: wrap; }
  .svc-text { flex: 1; min-width: 200px; }
  .svc .t { font-weight: 500; }
  .svc .d { font-size: 12px; color: var(--muted); }
  .led { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; }
  .led.on { background: var(--ok); box-shadow: 0 0 0 3px var(--ok-soft); }
</style>
