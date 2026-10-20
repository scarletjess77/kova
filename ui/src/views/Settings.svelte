<script lang="ts">
  import { disable as autostartOff, enable as autostartOn, isEnabled as autostartIsOn } from '@tauri-apps/plugin-autostart';
  import { openPath } from '@tauri-apps/plugin-opener';
  import { api, errText, type CoreKind, type CoreStatus, type GeoInfo, type GeoSource, type ServiceView, type Settings } from '../lib/api';
  import { fmtAgo, fmtBytes } from '../lib/format';
  import { app } from '../lib/state.svelte';
  import Donate from '../components/Donate.svelte';
  import Icon from '../components/Icon.svelte';

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
    await app.run(() => api.installCore(c), (tag) => `${c === 'xray' ? 'Xray' : 'Mihomo'} ${tag} installed`);
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
    await app.run(() => api.serviceInstall(), svc?.installed ? 'Kova service updated' : 'Kova service installed. TUN mode no longer needs administrator rights');
    svcBusy = null;
    await loadSvc();
  }
  async function removeSvc() {
    svcBusy = 'remove';
    await app.run(() => api.serviceUninstall(), 'Kova service removed');
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
  const GEO_LABEL: Record<string, string> = { default: 'Standard (MetaCubeX)', runetfreedom: 'Russia (runetfreedom)' };
  const geoSourceName = (id: string) => GEO_LABEL[id] ?? (id.startsWith('custom') ? 'Custom' : id);

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
    if (kind !== s?.core.geodata.kind) void setGeo({ kind } as GeoSource, 'Routing data downloaded and checked');
  }
  async function updateGeo() {
    geoBusy = true;
    await app.run(() => api.updateGeodata(), (m) => (m.verified ? 'Routing data updated and verified' : 'Routing data updated'));
    geoBusy = false;
    await loadGeo();
  }

  function port(v: string, put: (x: Settings, p: number) => void) {
    const p = Number(v);
    // Xray profiles also use the port above the proxy port, so 65535 itself is out.
    if (!Number.isInteger(p) || p < 1024 || p > 65534) { app.notify('Use a port between 1024 and 65534'); return; }
    void set((x) => put(x, p), 'Port saved');
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
    autostartIsOn().then((on) => {
      if (app.settings && app.settings.launch_at_login !== on) void set((x) => (x.launch_at_login = on));
    }).catch(() => {});
  });
</script>

<div class="page-head"><h1>Settings</h1><span class="sub">Changes apply right away. A running connection restarts when needed.</span></div>

{#if s}
  <div class="set">
    <!-- Connection settings on the left, the app itself on the right. Two independent
         columns so a tall card never leaves a gap beside a short one. -->
    <div class="col">
      <section class="card">
        <div class="card-h"><h2>Network</h2></div>
        <div class="form">
          <div class="fld full"><label for="st-port">Proxy port (HTTP + SOCKS5)</label>
            <input id="st-port" class="input mono" value={s.core.mixed_port} onchange={(e) => port(e.currentTarget.value, (x, p) => (x.core.mixed_port = p))} /></div>
          <div class="fld full"><label for="st-bypass">Skip the system proxy for</label>
            <input id="st-bypass" class="input mono small" value={s.proxy_bypass} onchange={(e) => set((x) => (x.proxy_bypass = e.currentTarget.value), 'Saved')} /></div>
        </div>
        <div class="rows">
          <div class="row-sw"><div><div class="t">Allow connections from LAN</div><div class="d">Share the proxy with phones and consoles on your network</div></div>
            <button class="switch" role="switch" aria-label="Allow LAN" aria-checked={s.core.allow_lan} onclick={() => toggle((x) => x.core.allow_lan, (x, v) => (x.core.allow_lan = v))}></button></div>
          <div class="row-sw"><div><div class="t">IPv6</div></div>
            <button class="switch" role="switch" aria-label="IPv6" aria-checked={s.core.ipv6} onclick={() => toggle((x) => x.core.ipv6, (x, v) => (x.core.ipv6 = v))}></button></div>
          <div class="row-sw"><div><div class="t">Sniff domains</div><div class="d">Reads TLS and HTTP hostnames so domain rules also work for raw IP connections</div></div>
            <button class="switch" role="switch" aria-label="Sniff domains" aria-checked={s.core.sniff} onclick={() => toggle((x) => x.core.sniff, (x, v) => (x.core.sniff = v))}></button></div>
        </div>
      </section>

      <section class="card">
        <div class="card-h"><h2>TUN mode</h2>
          <button class="switch aside-sw" role="switch" aria-label="TUN mode" aria-checked={s.core.tun.enabled} onclick={() => toggle((x) => x.core.tun.enabled, (x, v) => (x.core.tun.enabled = v))}></button></div>
        <div class="fld"><span class="lbl">Network stack</span>
          <div class="seg" role="group" aria-label="Network stack">
            {#each [['mixed', 'Mixed'], ['system', 'System'], ['gvisor', 'gVisor']] as [v, l] (v)}
              <button aria-pressed={s.core.tun.stack === v} onclick={() => set((x) => (x.core.tun.stack = v as Settings['core']['tun']['stack']))}>{l}</button>
            {/each}
          </div>
        </div>
        <p class="hint">TUN captures every app, including games and UDP. It needs system rights, which the Kova service provides so Kova itself never runs as administrator.</p>
        {#if svc?.supported}
          <div class="svc">
            <span class="led" class:on={svc.running}></span>
            <div class="svc-text">
              <div class="t">Kova service</div>
              <div class="d">
                {#if svc.running}Running · version {svc.version}{svc.core_version ? ` · Mihomo ${svc.core_version}` : ''}{svcOutdated ? ' · from another Kova build, update it' : ''}
                {:else if svc.installed}Installed but not running
                {:else}Not installed. Installing asks for administrator approval once.{/if}
              </div>
            </div>
            <div class="actions">
              {#if !svc.installed || !svc.running || svcOutdated}
                <button class="btn primary" class:busy={svcBusy === 'install'} disabled={svcBusy !== null} onclick={installSvc}>
                  {#if svcBusy === 'install'}<Icon name="refresh" />Waiting for approval…{:else}{svc.installed ? 'Repair or update' : 'Install'}{/if}</button>
              {/if}
              {#if svc.installed}
                <button class="btn ghost danger" class:busy={svcBusy === 'remove'} disabled={svcBusy !== null} onclick={removeSvc}>Remove</button>
              {/if}
            </div>
          </div>
        {/if}
        <div class="rows">
          <div class="row-sw"><div><div class="t">Set up routes automatically</div></div>
            <button class="switch" role="switch" aria-label="Auto route" aria-checked={s.core.tun.auto_route} onclick={() => toggle((x) => x.core.tun.auto_route, (x, v) => (x.core.tun.auto_route = v))}></button></div>
          <div class="row-sw"><div><div class="t">Strict route</div><div class="d">Stops DNS and traffic from leaking around the tunnel</div></div>
            <button class="switch" role="switch" aria-label="Strict route" aria-checked={s.core.tun.strict_route} onclick={() => toggle((x) => x.core.tun.strict_route, (x, v) => (x.core.tun.strict_route = v))}></button></div>
        </div>
      </section>

      <section class="card">
        <div class="card-h"><h2>DNS</h2></div>
        <div class="row-sw first"><div><div class="t">Fake-IP</div><div class="d">Fastest. Turn off if an app needs real IP answers.</div></div>
          <button class="switch" role="switch" aria-label="Fake-IP" aria-checked={s.core.dns.fake_ip} onclick={() => toggle((x) => x.core.dns.fake_ip, (x, v) => (x.core.dns.fake_ip = v))}></button></div>
        <div class="form">
          <div class="fld full"><label for="st-dns">Resolvers for proxied traffic</label>
            <input id="st-dns" class="input mono small" value={s.core.dns.nameservers.join(', ')} onchange={(e) => set((x) => (x.core.dns.nameservers = list(e.currentTarget.value)), 'Saved')} /></div>
          <div class="fld full"><label for="st-dns-d">Resolvers for direct traffic and server addresses</label>
            <input id="st-dns-d" class="input mono small" value={s.core.dns.direct_nameservers.join(', ')} onchange={(e) => set((x) => (x.core.dns.direct_nameservers = list(e.currentTarget.value)), 'Saved')} /></div>
        </div>
        <div class="row-sw"><div><div class="t">Use these settings for Clash profiles too</div><div class="d">Otherwise a Clash profile’s own DNS section is kept</div></div>
          <button class="switch" role="switch" aria-label="Override profile DNS" aria-checked={s.core.dns.override_profile} onclick={() => toggle((x) => x.core.dns.override_profile, (x, v) => (x.core.dns.override_profile = v))}></button></div>
      </section>

      <section class="card">
        <div class="card-h"><h2>Routing data</h2><span class="aside">GeoIP and GeoSite lists used by rules</span></div>
        <div class="seg" role="group" aria-label="Routing data source">
          {#each [['default', 'Standard'], ['runetfreedom', 'Russia'], ['custom', 'Custom']] as [v, l] (v)}
            <button aria-pressed={geoKind === v} disabled={geoBusy} onclick={() => pickGeo(v)}>{l}</button>
          {/each}
        </div>
        <p class="hint">
          {#if geoKind === 'default'}MetaCubeX lists: every country and the v2fly site categories. Updated weekly.
          {:else if geoKind === 'runetfreedom'}runetfreedom/russia-v2ray-rules-dat: the standard lists plus sites blocked in Russia. Needed for “Only blocked sites through the proxy”. About 90 MB, updated every 12 hours.
          {:else}Direct links to <span class="mono">geoip.dat</span> and <span class="mono">geosite.dat</span> in v2ray format. Kova checks that the files can be read, but can’t verify who published them.{/if}
        </p>
        {#if customOpen}
          <div class="form gap">
            <div class="fld full"><label for="geo-ip">geoip.dat address</label><input id="geo-ip" class="input mono small" bind:value={draft.geoip} placeholder="https://…/geoip.dat" /></div>
            <div class="fld full"><label for="geo-site">geosite.dat address</label><input id="geo-site" class="input mono small" bind:value={draft.geosite} placeholder="https://…/geosite.dat" /></div>
          </div>
          <div class="actions gap">
            <button class="btn primary" class:busy={geoBusy} disabled={geoBusy || !draft.geoip.trim() || !draft.geosite.trim()}
              onclick={() => setGeo({ kind: 'custom', geoip: draft.geoip.trim(), geosite: draft.geosite.trim() }, 'Custom routing data downloaded')}>{#if geoBusy}<Icon name="refresh" />Downloading…{:else}Download and use{/if}</button>
            <button class="btn ghost" onclick={() => (customOpen = false)}>Cancel</button>
          </div>
        {/if}
        {#if geo}
          <dl class="kv gap">
            <dt>Installed</dt>
            <dd>{geo.installed ? `${geoSourceName(geo.installed.source)} · ${fmtAgo(geo.installed.updated_at)}` : geo.files.length ? 'Bundled with Xray' : 'Not downloaded yet'}</dd>
            {#if geo.installed}<dt>Checksum</dt><dd>{geo.installed.verified ? 'Verified against the publisher' : 'Not available for this source'}</dd>{/if}
            {#each geo.files as f (f.name)}<dt class="mono">{f.name}</dt><dd>{fmtBytes(f.size)} · {f.categories} categories</dd>{/each}
          </dl>
          {#if geo.missing.length}
            <p class="hint warn-text">Missing for your presets: {geo.missing.join(', ')}. Pick a source that has them or turn the preset off.</p>
          {/if}
        {/if}
        {#if !customOpen}
          <div class="actions gap"><button class="btn" class:busy={geoBusy} disabled={geoBusy} onclick={updateGeo}><Icon name="refresh" />{geoBusy ? 'Downloading…' : 'Update now'}</button></div>
        {/if}
      </section>
    </div>
    <div class="col">
      <section class="card">
        <div class="card-h"><h2>Cores</h2>
          <button class="btn ghost aside-btn" class:busy={checking} onclick={() => loadCores(true)} disabled={checking}><Icon name="refresh" />Check for updates</button></div>
        {#each cores as c (c.core)}
          {@const name = c.core === 'xray' ? 'Xray' : 'Mihomo'}
          {@const update = c.latest && norm(c.latest) !== norm(c.installed)}
          <div class="core">
            <span class="ico">{name[0]}</span>
            <span class="t">{name} {c.installed ?? ''}</span>
            {#if !c.installed || update}
              <button class="btn primary" class:busy={installing === c.core} disabled={installing !== null} onclick={() => install(c.core)}>
                {#if installing === c.core}<Icon name="refresh" />Installing…{:else}{c.installed ? `Update to ${c.latest}` : 'Install'}{/if}</button>
            {:else}
              <span class="chip">{c.latest ? 'Up to date' : 'Installed'}</span>
            {/if}
            <span class="d">{c.core === 'xray' ? 'XTLS/Xray-core · XHTTP, Reality, bridged nodes' : 'MetaCubeX/mihomo · rules, groups, TUN, most protocols'}{!c.installed ? ' · not installed' : ''}</span>
          </div>
        {/each}
        <p class="hint">Downloaded from the official GitHub releases. Kova checks each file’s SHA-256 before installing it.</p>
      </section>

      <section class="card">
        <div class="card-h"><h2>App</h2></div>
        <div class="rows">
          <div class="row-sw"><div><div class="t">Launch at sign-in</div></div>
            <button class="switch" role="switch" aria-label="Launch at sign-in" aria-checked={s.launch_at_login} onclick={setAutostart}></button></div>
          <div class="row-sw"><div><div class="t">Start in the tray</div></div>
            <button class="switch" role="switch" aria-label="Start in the tray" aria-checked={s.start_minimized} onclick={() => toggle((x) => x.start_minimized, (x, v) => (x.start_minimized = v))}></button></div>
          <div class="row-sw"><div><div class="t">Connect on launch</div></div>
            <button class="switch" role="switch" aria-label="Connect on launch" aria-checked={s.connect_on_launch} onclick={() => toggle((x) => x.connect_on_launch, (x, v) => (x.connect_on_launch = v))}></button></div>
        </div>
        <div class="form gap">
          <div class="fld"><span class="lbl">Theme</span>
            <div class="seg" role="group" aria-label="Theme">
              {#each [['dark', 'Dark'], ['light', 'Light'], ['system', 'Match system']] as [v, l] (v)}
                <button aria-pressed={s.theme === v} onclick={() => set((x) => (x.theme = v as Settings['theme']))}>{l}</button>
              {/each}
            </div></div>
          <div class="fld"><label for="st-log">Core log level</label>
            <select id="st-log" class="input" value={s.core.log_level} onchange={(e) => set((x) => (x.core.log_level = e.currentTarget.value as Settings['core']['log_level']))}>
              <option value="debug">Debug</option><option value="info">Info</option><option value="warning">Warning</option><option value="error">Error</option><option value="silent">Silent</option>
            </select></div>
        </div>
        <div class="row-sw data"><div><div class="t">Data folder</div><div class="d mono selectable">{app.dataDir}</div></div>
          <button class="btn" onclick={() => app.run(() => openPath(app.dataDir))}><Icon name="folder" />Open</button></div>
        <div class="row-sw"><div><div class="t">Quit Kova</div><div class="d">Disconnects and restores your system proxy</div></div>
          <button class="btn danger" onclick={() => api.quit()}>Quit</button></div>
      </section>

      <section class="card">
        <div class="card-h"><h2>Updates</h2><span class="aside">Kova {svc?.app_version ?? ''}</span></div>
        {#if app.update}
          <div class="upd">
            <div class="t">Kova {app.update.version} is available</div>
            {#if app.update.date}<div class="d">Released {app.update.date.slice(0, 10)}</div>{/if}
            {#if app.update.body}<p class="notes">{app.update.body.slice(0, 400)}</p>{/if}
            {#if app.updateProgress !== null}
              <div class="bar"><i style="width:{Math.round(app.updateProgress * 100)}%"></i></div>
              <div class="d">{app.updateProgress < 1 ? `Downloading… ${Math.round(app.updateProgress * 100)}%` : 'Installing, Kova will restart'}</div>
            {:else}
              <div class="actions gap"><button class="btn primary" onclick={() => app.installUpdate()}><Icon name="down" />Install and restart</button></div>
              <p class="hint">Kova disconnects first. Windows asks for administrator approval because Kova is installed for all users.</p>
            {/if}
          </div>
        {/if}
        <div class="rows">
          <div class="row-sw"><div><div class="t">Check for updates on launch</div><div class="d">Releases are signed; Kova only installs updates signed with its key</div></div>
            <button class="switch" role="switch" aria-label="Check for updates on launch" aria-checked={s.check_updates} onclick={() => toggle((x) => x.check_updates, (x, v) => (x.check_updates = v))}></button></div>
        </div>
        <div class="actions gap"><button class="btn" class:busy={app.checkingUpdate} disabled={app.checkingUpdate || app.updateProgress !== null} onclick={() => app.checkForUpdate()}><Icon name="refresh" />Check now</button></div>
      </section>

      <section class="card" id="donate">
        <div class="card-h"><h2>Support Kova</h2><span class="aside">by scarletjess77</span></div>
        <Donate />
      </section>
    </div>
  </div>
{/if}

<style>
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
