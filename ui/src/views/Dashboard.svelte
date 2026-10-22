<script lang="ts">
  import { friendly } from '../lib/errors';
  import { onDestroy } from 'svelte';
  import { api, errText, type ExitInfo, type Mode } from '../lib/api';
  import { countryCode, daysUntil, displayName, fmtBytes, fmtDur, fmtSpeed } from '../lib/format';
  import { t, tn, tx } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';
  import Latency from '../components/Latency.svelte';
  import TrafficChart from '../components/TrafficChart.svelte';
  import DiagnoseModal from '../components/DiagnoseModal.svelte';

  let s = $derived(app.status);
  let settings = $derived(app.settings);
  let profile = $derived(app.activeProfile);

  // The group traffic starts in: GLOBAL in Global mode, else "Proxy" (our generated
  // profiles) or the first selector of an imported config.
  let mainGroup = $derived(
    (settings?.core.mode === 'global' && s.running ? app.proxies.groups.find((g) => g.name === 'GLOBAL') : undefined) ??
      app.proxies.groups.find((g) => g.name === 'Proxy') ??
      app.proxies.groups.find((g) => g.type === 'Selector' && g.name !== 'GLOBAL'),
  );
  // Running is not the same as protected: without the system proxy or a working TUN,
  // only apps pointed at the port by hand go through Kova.
  // On a phone only the VPN captures traffic; there is no system proxy.
  let capturing = $derived(!!settings && (s.tun_active || (!app.mobile && settings.system_proxy)));
  let chain = $derived.by(() => {
    const out: string[] = [];
    let name = mainGroup?.name;
    while (name && out.length < 6) {
      out.push(name);
      const g = app.proxies.groups.find((x) => x.name === name);
      name = g ? g.now || g.all[0] : undefined;
    }
    return out;
  });
  let endNode = $derived(chain.length ? chain[chain.length - 1] : undefined);
  let endInfo = $derived(endNode ? app.nodes[endNode] : undefined);
  let quick = $derived.by(() => {
    if (!mainGroup) return [];
    const groups = mainGroup.all.filter((n) => app.proxies.groups.some((g) => g.name === n));
    const nodes = mainGroup.all
      .filter((n) => !groups.includes(n) && app.delays[n] != null)
      .sort((a, b) => (app.delays[a] ?? 1e9) - (app.delays[b] ?? 1e9))
      .slice(0, 4);
    const fill = nodes.length ? [] : mainGroup.all.filter((n) => !groups.includes(n)).slice(0, 4);
    return [...groups.slice(0, 2), ...nodes, ...fill];
  });

  let now = $state(Date.now());
  const tick = setInterval(() => (now = Date.now()), 1000);

  let exit = $state<ExitInfo | null>(null);
  let exitErr = $state<string | null>(null);
  let exitLoading = $state(false);
  async function checkExit(retry = false) {
    exitLoading = true;
    try {
      exit = await api.exitInfo();
      exitErr = null;
    } catch (e) {
      exit = null;
      exitErr = errText(e);
      // Right after connecting the route may still be settling: try once more.
      if (retry) setTimeout(() => void checkExit(), 3000);
    }
    exitLoading = false;
  }
  $effect(() => {
    void s.running;
    if (!s.busy) void checkExit(true);
  });

  let connCount = $state(0);
  const poll = setInterval(async () => {
    if (!app.status.running) { connCount = 0; return; }
    try { connCount = (await api.getConnections()).connections.length; } catch { /* core restarting */ }
  }, 3000);
  onDestroy(() => { clearInterval(tick); clearInterval(poll); });

  const setMode = (m: Mode) => app.saveSettings((x) => (x.core.mode = m));
  let details = $state(false);
  let diagnosing = $state(false);

  // A subscription about to end or run out is worth saying before it happens.
  let subWarning = $derived.by(() => {
    const u = profile?.usage;
    if (!u) return null;
    if (u.expire) {
      const d = daysUntil(u.expire);
      if (d < 0) return t('{name} has expired. Renew it with your provider.', { name: profile!.name });
      if (d <= 3) return tn(Math.max(1, d), '{name} ends in {n} day. Renew it with your provider.', '{name} ends in {n} days. Renew it with your provider.', { name: profile!.name });
    }
    if (u.total && u.upload + u.download >= u.total * 0.9) {
      return t('{name} has used {pct}% of its traffic.', { name: profile!.name, pct: Math.min(100, Math.round(((u.upload + u.download) / u.total) * 100)) });
    }
    return null;
  });
</script>

<div class="page-head">
  <h1>{t('Dashboard')}</h1>
  {#if profile}<span class="sub">{profile.name} · {tn(profile.node_count, '{n} node', '{n} nodes')}</span>{/if}
  <div class="tools"><button class="btn ghost" onclick={() => (diagnosing = true)}><Icon name="pulse" />{t('What’s wrong?')}</button></div>
</div>
{#if subWarning}<div class="sub-warn"><Icon name="alert" />{subWarning}</div>{/if}
{#if diagnosing}<DiagnoseModal onclose={() => (diagnosing = false)} />{/if}

<div class="dash">
  <section class="card hero">
    <div class="hero-top">
      <button class="power" class:on={s.running} class:busy={s.busy || app.toggling} onclick={() => app.toggleConnection()} disabled={!profile || s.busy || app.toggling}
        aria-pressed={s.running} aria-label={s.running ? t('Disconnect') : t('Connect')}><Icon name="power" /></button>
      <div class="hero-text">
        <div class="eyebrow">{t('Status')}</div>
        <div class="state" class:off={!s.running && !s.busy}>{s.busy ? (s.running ? t('Switching…') : t('Connecting…')) : s.running ? (capturing ? t('Protected') : t('Proxy only')) : t('Disconnected')}</div>
        {#if !profile}
          <ol class="start">
            <li><b>{t('Add your servers')}</b><span>{app.mobile ? t('A subscription link, share links, or a QR code from your provider.') : t('A subscription link, share links, or a config file from your provider.')}</span></li>
            <li><b>{t('Press the power button')}</b><span>{t('Kova picks the core each server needs.')}</span></li>
            <li><b>{app.mobile ? t('Allow the VPN') : t('Choose what goes through Kova')}</b><span>{app.mobile ? t('Android asks once. Settings → VPN chooses which apps use it.') : t('System proxy covers browsers and most apps; TUN mode covers everything, games included.')}</span></li>
          </ol>
          <div><button class="btn primary" onclick={() => { app.go('profiles'); app.importOpen = true; }}><Icon name="plus" />{t('Add profile')}</button></div>
        {:else if chain.length}
          <div class="route">
            {#each chain.slice(0, -1) as c (c)}<span>{c}</span><span class="sep">›</span>{/each}
            {#if endNode}
              <span class="cc">{countryCode(endNode) ?? '··'}</span><span class="end">{displayName(endNode)}</span>
              {#if endInfo}<span class="chip">{endInfo.protocol} · {endInfo.transport}</span>{/if}
            {/if}
          </div>
        {/if}
        {#if profile}
          <div class="hero-meta">
            {#if s.running && s.core}
              <span class="chip acc"><Icon name="bolt" />{s.core === 'xray' ? 'Xray' : s.sidecar ? t('Mihomo + Xray bridge') : 'Mihomo'}</span>
              <span>{t('Connected for')} <span class="num">{fmtDur(now - (s.since ?? 0) * 1000)}</span></span>
              {#if app.mobile}<span class="speed num">↓ {fmtSpeed(app.down[app.down.length - 1])} · ↑ {fmtSpeed(app.up[app.up.length - 1])}</span>{/if}
            {:else if s.error}
              <span class="chip bad"><Icon name="alert" />{friendly(s.error)}</span>
            {:else}
              <span>{t('Your traffic goes out directly')}</span>
            {/if}
          </div>
          {#if s.running && s.warning}
            <div class="hero-meta"><span class="chip warn"><Icon name="alert" />{friendly(s.warning)}</span></div>
          {/if}
          {#if s.running && !capturing && settings}
            <div class="hero-meta"><span class="chip warn"><Icon name="alert" />{t('Apps aren’t routed yet. Turn on System proxy or TUN, or point an app at port {port}.', { port: settings.core.mixed_port })}</span></div>
          {/if}
          {#if s.running && s.dropped.length}
            <div class="hero-meta"><span class="chip warn"><Icon name="alert" />{tn(s.dropped.length, '{n} node skipped: {reason}', '{n} nodes skipped: {reason}', { reason: s.dropped[0][1] })}</span></div>
          {/if}
        {/if}
      </div>
    </div>
    {#if settings}
      <div class="hero-controls">
        <div class="fld"><span class="lbl">{t('Routing mode')}</span>
          <div class="seg" role="group" aria-label={t('Routing mode')}>
            {#each [['rule', tx('mode', 'Rule')], ['global', tx('mode', 'Global')], ['direct', tx('mode', 'Direct')]] as [m, l] (m)}
              <button aria-pressed={settings.core.mode === m} onclick={() => setMode(m as Mode)}>{l}</button>
            {/each}
          </div>
        </div>
        <!-- On a phone the VPN captures everything; there is nothing to choose. -->
        {#if !app.mobile}
        <div class="fld"><span class="lbl">{t('Capture traffic')}</span>
          <div class="togs">
            <div class="tog"><Icon name="monitor" /><span>{t('System proxy')}<small>{t('Apps that follow the system proxy')}</small></span>
              <button class="switch" role="switch" aria-label={t('System proxy')} aria-checked={settings.system_proxy}
                onclick={() => app.saveSettings((x) => (x.system_proxy = !x.system_proxy), settings.system_proxy ? t('System proxy off') : t('System proxy set to 127.0.0.1:{port}', { port: settings.core.mixed_port }))}></button></div>
            <div class="tog"><Icon name="shield" /><span>{t('TUN mode')}<small>{t('All apps, games and UDP')}</small></span>
              <button class="switch" role="switch" aria-label={t('TUN mode')} aria-checked={settings.core.tun.enabled}
                onclick={() => app.saveSettings((x) => (x.core.tun.enabled = !x.core.tun.enabled))}></button></div>
          </div>
        </div>
        {/if}
      </div>
    {/if}
  </section>

  {#if !app.mobile || details}
  <section class="card exit">
    <div class="card-h"><h2>{t('Exit point')}</h2><span class="aside">{s.running ? t('through the proxy') : t('without the proxy')}</span></div>
    <div>
      <div class="eyebrow">{t('Public IP')}</div>
      <div class="ip selectable">{exitLoading && !exit ? '…' : exit?.ip ?? '—'}</div>
    </div>
    <dl class="kv">
      <dt>{t('Location')}</dt><dd>{exit ? [exit.city, exit.country].filter(Boolean).join(', ') : exitErr ? t('Could not check') : '…'}</dd>
      {#if exitErr && !exit}<dt></dt><dd class="why selectable">{exitErr}</dd>{/if}
      <dt>{t('Network')}</dt><dd title={exit?.network}>{exit?.network || '—'}</dd>
      <dt>{t('Latency')}</dt><dd>{#if s.running && endNode}<Latency delay={app.delays[endNode]} />{:else}—{/if}</dd>
      {#if settings}<dt>DNS</dt><dd>{settings.core.dns.fake_ip ? 'Fake-IP' : 'Redir-host'} · {settings.core.dns.nameservers[0]?.replace(/^https:\/\//, '').replace(/\/dns-query$/, '') ?? '—'}</dd>{/if}
    </dl>
    <div class="foot">
      <button class="btn" class:busy={exitLoading} onclick={() => checkExit()} disabled={exitLoading}><Icon name="refresh" />{t('Check again')}</button>
      {#if exit}<button class="btn ghost" onclick={() => app.run(() => navigator.clipboard.writeText(exit!.ip), t('IP copied'))}><Icon name="copy" />{t('Copy IP')}</button>{/if}
    </div>
  </section>

  <section class="card">
    <div class="card-h"><h2>{t('Traffic')}</h2>
      <div class="legend">
        <span class="leg"><i style="background:var(--accent)"></i>{t('Download')} <b>{fmtSpeed(app.down[app.down.length - 1])}</b></span>
        <span class="leg"><i style="background:var(--upload)"></i>{t('Upload')} <b>{fmtSpeed(app.up[app.up.length - 1])}</b></span>
      </div>
    </div>
    <TrafficChart down={app.down} up={app.up} />
    <div class="traffic-foot"><span>{t('This session')} ↓ <b>{fmtBytes(app.downTotal)}</b></span><span>↑ <b>{fmtBytes(app.upTotal)}</b></span></div>
  </section>
  {/if}

  <section class="card">
    <div class="card-h"><h2>{t('Quick switch')}</h2>{#if mainGroup}<span class="aside">{t('{name} group', { name: mainGroup.name })}</span>{/if}</div>
    {#if quick.length && mainGroup}
      <div class="qs">
        {#each quick as name (name)}
          {@const isGroup = app.proxies.groups.some((g) => g.name === name)}
          <button aria-pressed={mainGroup.now === name} onclick={() => app.select(mainGroup!.name, name)}>
            <span class="cc">{#if isGroup}<Icon name="bolt" />{:else}{countryCode(name) ?? '··'}{/if}</span>
            <span class="nm">{displayName(name)}<span class="pr"> · {isGroup ? t('now {name}', { name: displayName(app.resolve(name)) }) : (app.nodes[name]?.protocol ?? '')}</span></span>
            <Latency delay={app.delayOf(name)} testing={app.testing.has(app.resolve(name))} />
          </button>
        {/each}
      </div>
      <button class="btn ghost more" onclick={() => app.go('proxies')}>{t('All nodes and groups')}</button>
    {:else}
      <p class="muted">{t('Nodes appear here once a profile is added.')}</p>
    {/if}
  </section>

  {#if app.mobile}
    <!-- On a phone the essentials come first; the rest is a tap away. -->
    <button class="btn ghost details-btn" aria-expanded={details} onclick={() => (details = !details)}>{details ? t('Fewer details') : t('Exit point, traffic and more')}</button>
  {/if}
  {#if !app.mobile || details}
  <section class="tiles">
    <div class="card tile"><span class="eyebrow">{t('Connections')}</span><span class="v">{connCount}</span><span class="d">{t('open right now')}</span></div>
    <div class="card tile"><span class="eyebrow">{t('Downloaded')}</span><span class="v">{fmtBytes(app.downTotal)}</span><span class="d">{t('this session')}</span></div>
    <div class="card tile"><span class="eyebrow">{t('Uploaded')}</span><span class="v">{fmtBytes(app.upTotal)}</span><span class="d">{t('this session')}</span></div>
    <div class="card tile"><span class="eyebrow">{t('Core memory')}</span><span class="v">{app.memory ? fmtBytes(app.memory) : '—'}</span><span class="d">{s.running ? (s.core === 'xray' ? t('not reported by Xray') : 'Mihomo') : t('not running')}</span></div>
  </section>
  {/if}
</div>

<style>
  .sub-warn { display: flex; align-items: center; gap: 8px; margin: -6px 0 14px; padding: 10px 12px; border-radius: var(--r); background: var(--warn-soft); color: var(--warn); font-size: 13px; }
  .sub-warn :global(.i) { width: 16px; height: 16px; flex: none; }
  .details-btn { justify-self: center; grid-column: 1 / -1; }
  .speed { color: var(--fg); }
  .start { margin: 4px 0 12px; padding-left: 20px; display: flex; flex-direction: column; gap: 8px; font-size: 13px; color: var(--muted); }
  .start li::marker { color: var(--faint); font: 600 12px var(--f-mono); }
  .start b { display: block; color: var(--fg); font-weight: 600; }
  .dash { display: grid; grid-template-columns: minmax(0, 1.55fr) minmax(0, 1fr); gap: 14px; }
  .tiles { grid-column: 1 / -1; display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; }
  @container (max-width: 860px) { .dash { grid-template-columns: minmax(0, 1fr); } }
  @container (max-width: 700px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .hero { display: flex; flex-direction: column; gap: 20px; padding: 20px; }
  .hero-top { display: flex; gap: 22px; align-items: center; }
  .hero-text { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .power { width: 96px; height: 96px; border-radius: 50%; border: 1.5px solid var(--line-2); background: var(--sunk); color: var(--muted); display: grid; place-items: center; flex: none; position: relative; transition: background .2s, color .2s, border-color .2s; }
  .power :global(.i) { width: 36px; height: 36px; stroke-width: 2; }
  /* Hover only with a mouse: on a touch screen it sticks after the tap. */
  @media (hover: hover) { .power:not(.on):hover:not(:disabled) { border-color: var(--accent); color: var(--fg); } }
  .power:disabled { cursor: default; }
  .power.on { background: var(--accent); border-color: var(--accent); color: var(--accent-fg); }
  .power.on::before { content: ''; position: absolute; inset: -7px; border-radius: 50%; border: 1.5px solid var(--accent); opacity: .3; }
  .power.busy::before { content: ''; position: absolute; inset: -7px; border-radius: 50%; border: 2px solid transparent; border-top-color: var(--accent); animation: spin .8s linear infinite; }
  .state { font: 700 26px/1.1 var(--f-display); letter-spacing: -.01em; }
  .state.off { color: var(--muted); }
  .route { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 13.5px; }
  .route .sep { color: var(--faint); }
  .route .end { font-weight: 600; }
  .hero-meta { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; color: var(--muted); font-size: 12.5px; }
  .hero-meta .chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .hero-controls { display: grid; grid-template-columns: auto 1fr; gap: 16px 28px; align-items: start; padding-top: 18px; border-top: 1px solid var(--line); }
  @container (max-width: 560px) { .hero-controls { grid-template-columns: 1fr; } .hero-top { flex-direction: column; align-items: flex-start; } }
  .togs { display: flex; gap: 10px; flex-wrap: wrap; }

  /* Phone: the connection is the page. Big centred button, status beneath it. */
  :global(.app.mobile) .hero { padding: 28px 18px 20px; }
  :global(.app.mobile) .hero-top { flex-direction: column; align-items: center; text-align: center; gap: 18px; }
  :global(.app.mobile) .hero-text { align-items: center; width: 100%; }
  :global(.app.mobile) .power { width: 148px; height: 148px; }
  :global(.app.mobile) .power :global(.i) { width: 52px; height: 52px; }
  :global(.app.mobile) .state { font-size: 30px; }
  :global(.app.mobile) .route, :global(.app.mobile) .hero-meta { justify-content: center; }
  :global(.app.mobile) .hero-meta .chip { text-align: left; }
  :global(.app.mobile) .hero-controls { grid-template-columns: minmax(0, 1fr); }
  :global(.app.mobile) .hero-controls .seg { display: flex; }
  :global(.app.mobile) .hero-controls .seg button { flex: 1; }
  .tog { display: flex; align-items: center; gap: 10px; padding: 6px 8px 6px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--win); }
  .tog :global(.i) { color: var(--muted); }
  .tog span { font-size: 13px; font-weight: 500; }
  .tog small { display: block; color: var(--faint); font-size: 11px; font-weight: 400; }
  .exit { display: flex; flex-direction: column; gap: 14px; }
  .ip { font: 500 22px var(--f-mono); letter-spacing: -.01em; }
  .why { color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
  .foot { margin-top: auto; display: flex; gap: 8px; flex-wrap: wrap; }
  .legend { display: flex; gap: 18px; flex-wrap: wrap; margin-left: auto; }
  .leg { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--muted); }
  .leg i { width: 10px; height: 3px; border-radius: 2px; display: block; }
  .leg b { font: 500 14px var(--f-mono); color: var(--fg); }
  .traffic-foot { display: flex; gap: 20px; flex-wrap: wrap; margin-top: 10px; color: var(--muted); font-size: 12.5px; }
  .traffic-foot b { color: var(--fg); font-weight: 500; font-family: var(--f-mono); }
  .qs { display: flex; flex-direction: column; gap: 4px; }
  .qs button { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 1px solid transparent; border-radius: 8px; background: none; text-align: left; min-width: 0; }
  @media (hover: hover) { .qs button:hover { background: var(--sunk); } }
  .qs button[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); }
  .qs .nm { font-weight: 500; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .qs .pr { color: var(--muted); font-size: 12px; font-weight: 400; }
  .more { margin-top: 8px; }
  .tile { display: flex; flex-direction: column; gap: 4px; }
  .tile .v { font: 500 22px var(--f-mono); letter-spacing: -.02em; }
  .tile .d { font-size: 12px; color: var(--muted); }
</style>
