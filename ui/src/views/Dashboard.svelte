<script lang="ts">
  import { friendly } from '../lib/errors';
  import { onDestroy } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api, errText, type CorePreference, type ExitInfo, type Mode, type Preset, type Quality } from '../lib/api';
  import { countryLabel, daysUntil, displayName, fmtBytes, fmtDur, fmtSpeed } from '../lib/format';
  import { t, tn, tx } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';
  import Latency from '../components/Latency.svelte';
  import TrafficChart from '../components/TrafficChart.svelte';
  import DiagnoseModal from '../components/DiagnoseModal.svelte';
  import { FIXED_CARDS, move, type CardId } from '../lib/layout';
  import { sortable } from '../lib/sortable';
  import { pictureStyle } from '../lib/pictures';

  let s = $derived(app.status);
  let settings = $derived(app.settings);
  let profile = $derived(app.activeProfile);

  // First start: where the user is decides which local sites go direct. It starts from
  // the system language; a Russian user with an English system picks it here.
  type Region = 'direct-ru' | 'direct-ir' | 'direct-cn' | 'none';
  const LOCAL: Preset[] = ['direct-ru', 'direct-ir', 'direct-cn', 'proxy-blocked-ru'];
  let region = $derived.by((): Region => {
    const p = settings?.core.presets ?? [];
    if (p.includes('direct-ru') || p.includes('proxy-blocked-ru')) return 'direct-ru';
    return p.includes('direct-ir') ? 'direct-ir' : p.includes('direct-cn') ? 'direct-cn' : 'none';
  });
  function setRegion(r: Region) {
    if (r === region) return;
    void app.saveSettings((x) => { x.core.presets = [...x.core.presets.filter((p) => !LOCAL.includes(p)), ...(r === 'none' ? [] : [r])]; });
  }

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
  const tick = setInterval(() => { if (!document.hidden) now = Date.now(); }, 1000);

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

  // Connection quality: ten probes, on request. A result belongs to the connection it
  // was measured on, so connecting or disconnecting clears it.
  let quality = $state<Quality | null>(null);
  let qualityBusy = $state(false);
  async function checkQuality() {
    qualityBusy = true;
    quality = (await app.run(() => api.checkQuality())) ?? quality;
    qualityBusy = false;
  }
  $effect(() => {
    void s.running;
    quality = null;
  });
  const lossText = (q: Quality) => `${Math.round(q.loss)}%`;

  let connCount = $state(0);
  const poll = setInterval(async () => {
    if (document.hidden) return;
    if (!app.status.running) { connCount = 0; return; }
    try { connCount = (await api.getConnections()).connections.length; } catch { /* core restarting */ }
  }, 3000);
  onDestroy(() => { clearInterval(tick); clearInterval(poll); });

  const setMode = (m: Mode) => app.saveSettings((x) => (x.core.mode = m));
  // The active profile's core; a running connection restarts on it.
  const CORE_NAME: Record<CorePreference, string> = { auto: '', mihomo: 'Mihomo', xray: 'Xray' };
  const setCore = (c: CorePreference) => profile && profile.core !== c &&
    app.run(() => api.editProfile(profile!.id, { core: c }), c === 'auto' ? t('Kova picks the core for each server') : t('Now running on {core}', { core: CORE_NAME[c] }));
  let details = $state(false);
  let bannerPic = $derived(app.settings?.look?.pictures?.banner);
  let bannerUrl = $derived(app.pictureUrl(bannerPic?.file));

  // The user's layout. Cards below "More details" wait behind that button; while
  // arranging, every card shows, hidden ones faded.
  let CARD_NAME = $derived<Record<CardId, string>>({
    connect: t('Connect and status'), exit: t('Exit point'), quality: t('Connection quality'), traffic: t('Traffic'),
    quick: t('Quick switch'), tiles: t('Session numbers'), more: t('More details'),
  });
  /** Each card's own width, wherever it goes: wide, narrow, or the whole row. */
  const KIND: Record<CardId, 'wide' | 'narrow' | 'full'> = { connect: 'wide', exit: 'narrow', quality: 'full', traffic: 'wide', quick: 'narrow', tiles: 'full', more: 'full' };
  let cards = $derived(app.layout.cards);
  let fold = $derived(cards.findIndex((c) => c.id === 'more'));
  let below = $derived(cards.slice(fold + 1).some((c) => !c.hidden));
  let shown = $derived(app.editingLayout ? cards : cards.filter((c, i) => (c.id === 'more' ? below : !c.hidden && (i < fold || details))));
  // Rows: a wide and a narrow card side by side share one, in the user's order; a half
  // card with no partner next to it takes the whole row, so no row has a gap.
  let place = $derived.by(() => {
    const out: Partial<Record<CardId, string>> = {};
    for (let i = 0; i < shown.length; ) {
      const a = shown[i], b = shown[i + 1];
      const ka = KIND[a.id], kb = b ? KIND[b.id] : 'full';
      if (ka !== 'full' && kb !== 'full' && ka !== kb) {
        out[a.id] = ka === 'wide' ? 'l-wide' : 'l-narrow';
        out[b.id] = kb === 'wide' ? 'r-wide' : 'r-narrow';
        i += 2;
      } else {
        out[a.id] = 'all';
        i += 1;
      }
    }
    return out;
  });
  const moveCard = (from: number, to: number) => app.saveLayout({ ...app.layout, cards: move(app.layout.cards, from, to) });
  const toggleCard = (id: CardId) => app.saveLayout({ ...app.layout, cards: app.layout.cards.map((c) => (c.id === id ? { id, ...(c.hidden ? {} : { hidden: true }) } : c)) });
  let diagnosing = $state(false);

  // Android may stop a background VPN to save battery; say it once, until allowed or put away.
  let batteryHint = $state((() => { try { return localStorage.getItem('kova.batteryHint') !== 'hidden'; } catch { return true; } })());
  function hideBatteryHint() {
    batteryHint = false;
    try { localStorage.setItem('kova.batteryHint', 'hidden'); } catch { /* the hint comes back next time */ }
  }

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
  <div class="tools">
    {#if profile?.support_url}<button class="btn ghost" onclick={() => app.run(() => openUrl(profile!.support_url!))}><Icon name="external" />{t('Provider support')}</button>{/if}
    <button class="btn ghost" onclick={() => (diagnosing = true)}><Icon name="pulse" />{t('What’s wrong?')}</button>
    {#if !app.editingLayout}<button class="btn ghost" onclick={() => (app.editingLayout = true)}><Icon name="edit" />{t('Edit layout')}</button>{/if}
  </div>
</div>
{#if subWarning}<div class="sub-warn"><Icon name="alert" />{subWarning}</div>{/if}
{#if app.mobile && !app.tv && !app.batteryFree && batteryHint && s.running}
  <div class="sub-warn"><Icon name="alert" /><span class="grow">{t('Android may stop Kova in the background to save battery.')}</span>
    <button class="btn" onclick={() => app.run(() => api.allowBackground())}>{t('Allow')}</button>
    <button class="x" aria-label={t('Dismiss')} onclick={hideBatteryHint}><Icon name="x" /></button></div>
{/if}
{#if profile?.announce}<div class="announce" role="note"><Icon name="globe" /><span><b>{t('From the provider')}:</b> <span class="selectable">{profile.announce}</span></span></div>{/if}
{#if diagnosing}<DiagnoseModal onclose={() => (diagnosing = false)} />{/if}

{#snippet cardConnect()}
  <section class="card hero" class:guide={!profile} style={bannerPic && bannerUrl ? pictureStyle(bannerPic, bannerUrl) : undefined}>
    <div class="hero-top">
      <button class="power" class:on={s.running} class:busy={s.busy || app.toggling} onclick={() => app.toggleConnection()} disabled={!profile || s.busy || app.toggling}
        aria-pressed={s.running} aria-label={s.running ? t('Disconnect') : t('Connect')}><Icon name="power" /></button>
      <div class="hero-text">
        <div class="eyebrow">{t('Status')}</div>
        <div class="state" class:off={!s.running && !s.busy}>{s.busy ? (s.running ? t('Switching…') : t('Connecting…')) : s.running ? (capturing ? t('Protected') : t('Proxy only')) : t('Disconnected')}</div>
        {#if !profile}
          <ol class="start">
            <li><b>{t('Where are you?')}</b><span>{t('Local sites go direct, the rest through the server. Rules can change it later.')}</span>
              <div class="seg region" role="group" aria-label={t('Where are you?')}>
                <button aria-pressed={region === 'direct-ru'} onclick={() => setRegion('direct-ru')}>{t('Russia')}</button>
                <button aria-pressed={region === 'direct-ir'} onclick={() => setRegion('direct-ir')}>{t('Iran')}</button>
                <button aria-pressed={region === 'direct-cn'} onclick={() => setRegion('direct-cn')}>{t('China')}</button>
                <button aria-pressed={region === 'none'} onclick={() => setRegion('none')}>{t('Elsewhere')}</button>
              </div></li>
            <li><b>{t('Add your servers')}</b><span>{app.mobile ? t('A subscription link, share links, or a QR code from your provider.') : t('A subscription link, share links, or a config file from your provider.')}</span></li>
            <li><b>{t('Press the power button')}</b><span>{t('Kova picks the core each server needs.')}</span></li>
            <li><b>{app.mobile ? t('Allow the VPN') : t('Choose what goes through Kova')}</b><span>{app.mobile ? t('Android asks once. Settings → VPN chooses which apps use it.') : t('System proxy covers browsers and most apps; TUN mode covers everything, games included.')}</span></li>
          </ol>
          <div><button class="btn primary" onclick={() => { app.go('profiles'); app.importOpen = true; }}><Icon name="plus" />{t('Add profile')}</button></div>
        {:else if chain.length}
          <div class="route">
            {#each chain.slice(0, -1) as c (c)}<span>{c}</span><span class="sep">›</span>{/each}
            {#if endNode}
              <span class="cc">{countryLabel(endNode, app.flags)}</span><span class="end">{displayName(endNode)}</span>
              {#if endInfo}<span class="chip">{endInfo.protocol} · {endInfo.transport}</span>{/if}
            {/if}
          </div>
        {/if}
        {#if profile}
          <div class="hero-meta">
            {#if s.running && s.core}
              <span class="chip acc"><Icon name="bolt" />{s.core === 'xray' ? 'Xray' : s.bridged.length ? t('Mihomo + Xray bridge') : 'Mihomo'}</span>{#if settings?.core.anti_dpi && s.sidecar}<span class="chip">{t('Anti-DPI')}</span>{/if}
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
        <div class="col">
        <div class="fld"><span class="lbl">{t('Routing mode')}</span>
          <div class="seg" role="group" aria-label={t('Routing mode')}>
            {#each [['rule', tx('mode', 'Rule')], ['global', tx('mode', 'Global')], ['direct', tx('mode', 'Direct')]] as [m, l] (m)}
              <button aria-pressed={settings.core.mode === m} onclick={() => setMode(m as Mode)}>{l}</button>
            {/each}
          </div>
        </div>
        {#if profile}
          <div class="fld"><span class="lbl">{t('Core')}</span>
            <div class="seg" role="group" aria-label={t('Core')}>
              <!-- On a phone the VPN needs Mihomo in front; Xray still runs what only it supports. -->
              {#each [['auto', t('Automatic')], ['mihomo', 'Mihomo'], ...(!app.mobile || profile.core === 'xray' ? [['xray', 'Xray']] : [])] as [c, l] (c)}
                <button aria-pressed={profile.core === c} onclick={() => setCore(c as CorePreference)}>{l}</button>
              {/each}
            </div>
          </div>
        {/if}
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
{/snippet}
{#snippet cardExit()}
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
      <dt>{t('Latency')}</dt><dd>{#if s.running && endNode}<Latency delay={app.delays[endNode]} dead={app.isDead(endNode)} />{:else}—{/if}</dd>
      {#if settings}<dt>DNS</dt><dd>{settings.core.dns.fake_ip ? 'Fake-IP' : 'Redir-host'} · {settings.core.dns.nameservers[0]?.replace(/^https:\/\//, '').replace(/\/dns-query$/, '') ?? '—'}</dd>{/if}
    </dl>
    <div class="foot">
      <button class="btn" class:busy={exitLoading} onclick={() => checkExit()} disabled={exitLoading}><Icon name="refresh" />{t('Check again')}</button>
      {#if exit}<button class="btn ghost" onclick={() => app.run(() => navigator.clipboard.writeText(exit!.ip), t('IP copied'))}><Icon name="copy" />{t('Copy IP')}</button>{/if}
    </div>
  </section>
{/snippet}
{#snippet cardQuality()}
  <section class="card quality">
    <div class="card-h"><h2>{t('Connection quality')}</h2><span class="aside">{s.running ? t('through the proxy') : t('without the proxy')}</span></div>
    {#if quality}
      <div class="qgrid">
        <div><span class="eyebrow">{t('Latency')}</span><span class="v">{quality.median != null ? t('{n} ms', { n: quality.median }) : '—'}</span>
          <span class="d">{quality.min != null && quality.max != null ? t('{min} to {max} ms', { min: quality.min, max: quality.max }) : t('No answer')}</span></div>
        <div><span class="eyebrow">{t('Jitter')}</span><span class="v" class:warn={(quality.jitter ?? 0) > 30}>{quality.jitter != null ? t('{n} ms', { n: quality.jitter }) : '—'}</span>
          <span class="d">{t('change between requests')}</span></div>
        <div><span class="eyebrow">{t('Loss')}</span><span class="v" class:warn={quality.loss > 0}>{lossText(quality)}</span>
          <span class="d">{t('{n} of {total} answered', { n: quality.answered, total: quality.sent })}</span></div>
        <div><span class="eyebrow">{t('Connecting')}</span><span class="v">{quality.connect != null ? t('{n} ms', { n: quality.connect }) : '—'}</span>
          <span class="d">{t('first request, with the setup')}</span></div>
      </div>
    {:else}
      <p class="muted qhint">{t('One connection to the latency test address, then ten requests over it: each is a single round trip, like ping. Shows the latency, its spread, jitter and lost requests.')}</p>
    {/if}
    <div class="foot">
      <button class="btn" class:busy={qualityBusy} onclick={checkQuality} disabled={qualityBusy || s.busy}><Icon name="bolt" />{qualityBusy ? t('Checking…') : quality ? t('Check again') : t('Check quality')}</button>
    </div>
  </section>
{/snippet}
{#snippet cardTraffic()}
  <section class="card">
    <div class="card-h"><h2>{t('Traffic')}</h2>
      <div class="legend">
        <span class="leg"><i style="background:var(--accent)"></i>{t('Download')} <b>{fmtSpeed(app.down[app.down.length - 1])}</b></span>
        <span class="leg"><i style="background:var(--upload)"></i>{t('Upload')} <b>{fmtSpeed(app.up[app.up.length - 1])}</b></span>
      </div>
    </div>
    <TrafficChart down={app.down} up={app.up} />
  </section>
{/snippet}
{#snippet cardQuick()}
  <section class="card">
    <div class="card-h"><h2>{t('Quick switch')}</h2>{#if mainGroup}<span class="aside">{t('{name} group', { name: mainGroup.name })}</span>{/if}</div>
    {#if quick.length && mainGroup}
      <div class="qs">
        {#each quick as name (name)}
          {@const isGroup = app.proxies.groups.some((g) => g.name === name)}
          <button aria-pressed={mainGroup.now === name} onclick={() => app.select(mainGroup!.name, name)}>
            <span class="cc">{#if isGroup}<Icon name="bolt" />{:else}{countryLabel(name, app.flags)}{/if}</span>
            <span class="nm">{displayName(name)}<span class="pr"> · {isGroup ? t('now {name}', { name: displayName(app.resolve(name)) }) : (app.nodes[name]?.protocol ?? '')}</span></span>
            <Latency delay={app.delayOf(name)} dead={app.isDead(name)} testing={app.testing.has(app.resolve(name))} />
          </button>
        {/each}
      </div>
      <button class="btn ghost more" onclick={() => app.go('proxies')}>{t('All nodes and groups')}</button>
    {:else}
      <p class="muted">{t('Nodes appear here once a profile is added.')}</p>
    {/if}
  </section>
{/snippet}
{#snippet cardTiles()}
  <section class="tiles">
    <div class="card tile wide"><span class="eyebrow">{t('This session')}</span><span class="v pair"><span>↓ {fmtBytes(app.downTotal)}</span><span class="up">↑ {fmtBytes(app.upTotal)}</span></span><span class="d">{t('Downloaded')} · {t('Uploaded')}</span></div>
    <div class="card tile"><span class="eyebrow">{t('Connections')}</span><span class="v">{connCount}</span><span class="d">{t('open right now')}</span></div>
    <div class="card tile"><span class="eyebrow">{t('Core memory')}</span><span class="v">{app.memory ? fmtBytes(app.memory) : '—'}</span><span class="d">{s.running ? (s.core === 'xray' ? t('not reported by Xray') : 'Mihomo') : t('not running')}</span></div>
  </section>
{/snippet}

<div class="dash" class:editing={app.editingLayout} use:sortable={{ enabled: app.editingLayout, onmove: moveCard }}>
  {#each shown as c (c.id)}
    {#if c.id === 'more'}
      {#if app.editingLayout}
        <div class="slot all fold" data-sort-id="more">
          <button class="handle" data-handle aria-label={t('Move {name}', { name: CARD_NAME.more })} title={t('Drag to move')}><Icon name="grip" /></button>
          <span><b>{CARD_NAME.more}</b> · {t('cards below this line wait behind a button')}</span>
        </div>
      {:else}
        <button class="btn ghost details-btn" data-sort-id="more" aria-expanded={details} onclick={() => (details = !details)}>{details ? t('Fewer details') : t('Exit point, traffic and more')}</button>
      {/if}
    {:else}
      <div class="slot {place[c.id]}" data-sort-id={c.id} class:off={c.hidden}>
        {#if app.editingLayout}
          <div class="edit-h">
            <button class="handle" data-handle aria-label={t('Move {name}', { name: CARD_NAME[c.id] })} title={t('Drag to move')}><Icon name="grip" /></button>
            <b>{CARD_NAME[c.id]}</b>
            {#if !FIXED_CARDS.includes(c.id)}
              <button class="eye" onclick={() => toggleCard(c.id)} aria-pressed={!c.hidden} aria-label={c.hidden ? t('Show {name}', { name: CARD_NAME[c.id] }) : t('Hide {name}', { name: CARD_NAME[c.id] })} title={c.hidden ? t('Show') : t('Hide')}><Icon name={c.hidden ? 'eye-off' : 'eye'} /></button>
            {/if}
          </div>
        {/if}
        {#if c.id === 'connect'}{@render cardConnect()}
        {:else if c.id === 'exit'}{@render cardExit()}
        {:else if c.id === 'quality'}{@render cardQuality()}
        {:else if c.id === 'traffic'}{@render cardTraffic()}
        {:else if c.id === 'quick'}{@render cardQuick()}
        {:else if c.id === 'tiles'}{@render cardTiles()}{/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .sub-warn { display: flex; align-items: center; gap: 8px; margin: -6px 0 14px; padding: 10px 12px; border-radius: var(--r); background: var(--warn-soft); color: var(--warn); font-size: 13px; }
  .sub-warn :global(.i) { width: 16px; height: 16px; flex: none; }
  .sub-warn .grow { flex: 1; min-width: 0; }
  .sub-warn .x { color: inherit; }
  .announce { display: flex; align-items: flex-start; gap: 8px; margin: -6px 0 14px; padding: 10px 12px; border-radius: var(--r); background: var(--panel); border: 1px solid var(--line); font-size: 13px; overflow-wrap: anywhere; }
  .announce :global(.i) { width: 16px; height: 16px; flex: none; margin-top: 1px; color: var(--muted); }
  .details-btn { justify-self: center; grid-column: 1 / -1; }
  .slot { display: flex; flex-direction: column; min-width: 0; gap: 8px; }
  .slot > :global(section) { flex: 1; }
  /* Three columns, narrow · middle · narrow: a wide card spans two of them, so it is the
     same width whether it stands left or right of its narrow partner. */
  .slot.all { grid-column: 1 / -1; }
  .slot.l-wide { grid-column: 1 / span 2; }
  .slot.r-narrow { grid-column: 3; }
  .slot.l-narrow { grid-column: 1; }
  .slot.r-wide { grid-column: 2 / span 2; }
  /* The exit card keeps its own height beside the taller connect card. */
  .slot:has(> .exit) { align-self: start; }
  .slot.off > :global(section) { opacity: .4; }
  .edit-h { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--muted); }
  .edit-h b { font-weight: 500; flex: 1; min-width: 0; }
  .dash .handle, .dash .eye { padding: 4px; border: 0; background: none; color: var(--faint); border-radius: 6px; }
  .dash .handle { cursor: grab; touch-action: none; }
  .dash.editing .slot { padding: 8px; border: 1px dashed var(--line-2); border-radius: calc(var(--r) + 4px); }
  .dash :global(.dragging) { opacity: .6; }
  .dash :global(.drop-target) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .fold { flex-direction: row; align-items: center; gap: 8px; font-size: 12px; color: var(--muted); }
  .fold b { font-weight: 500; color: var(--fg); }
  .speed { color: var(--fg); }
  .start { margin: 4px 0 12px; padding-inline-start: 20px; text-align: start; align-self: stretch; display: flex; flex-direction: column; gap: 8px; font-size: 13px; color: var(--muted); }
  .start li::marker { color: var(--faint); font: 500 12px var(--f-mono); }
  .start b { display: block; color: var(--fg); font-weight: 500; }
  .start .region { display: flex; width: fit-content; margin-top: 8px; }
  /* The side cards grow with the window only up to a width they can use. */
  .dash { display: grid; grid-template-columns: clamp(280px, 39%, 420px) minmax(0, 1fr) clamp(280px, 39%, 420px); gap: 14px; }
  .tiles { grid-column: 1 / -1; display: grid; grid-template-columns: minmax(0, 2fr) repeat(2, minmax(0, 1fr)); gap: 14px; }
  @container (max-width: 860px) { .dash { grid-template-columns: minmax(0, 1fr); } .dash > .slot.slot { grid-column: 1 / -1; } }
  @container (max-width: 700px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } .tile.wide { grid-column: 1 / -1; } }
  .hero { display: flex; flex-direction: column; gap: 20px; padding: 20px; container: hero / inline-size; }
  .hero-top { display: flex; gap: 24px; align-items: center; }
  .hero-text { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .power { width: 96px; height: 96px; border-radius: 50%; border: 1.5px solid var(--line-2); background: var(--sunk); color: var(--muted); display: grid; place-items: center; flex: none; position: relative; transition: background .2s, color .2s, border-color .2s; }
  .power :global(.i) { width: 36px; height: 36px; stroke-width: 2; }
  /* Hover only with a mouse: on a touch screen it sticks after the tap. */
  @media (hover: hover) { .power:not(.on):hover:not(:disabled) { border-color: var(--accent); color: var(--fg); } }
  .power:disabled { cursor: default; }
  .power.on { background: var(--accent); border-color: var(--accent); color: var(--accent-fg); }
  .power.on::before { content: ''; position: absolute; inset: -8px; border-radius: 50%; border: 1.5px solid var(--accent); opacity: .3; }
  .power.busy::before { content: ''; position: absolute; inset: -8px; border-radius: 50%; border: 2px solid transparent; border-top-color: var(--accent); animation: spin .8s linear infinite; }
  .state { font: 500 26px/1.1 var(--f-display); }
  .state.off { color: var(--muted); }
  .route { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 14px; }
  .route .sep { color: var(--faint); }
  .route .end { font-weight: 500; }
  .hero-meta { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; color: var(--muted); font-size: 12px; }
  .hero-meta .chip { max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .hero-controls .col { display: flex; flex-direction: column; gap: 14px; min-width: 0; }
  .hero-controls { display: grid; grid-template-columns: auto 1fr; gap: 16px 28px; align-items: start; padding-top: 16px; border-top: 1px solid var(--line); }
  @container hero (max-width: 420px) { .hero-controls { grid-template-columns: 1fr; } .hero-top { flex-direction: column; align-items: flex-start; } }
  /* The first-start guide is long: in a narrow desktop card the power button goes above it, so
     the steps get the full width instead of wrapping into a tall column. */
  @container hero (max-width: 640px) {
    :global(.app:not(.mobile)) .guide .hero-top { flex-direction: column; align-items: flex-start; gap: 14px; }
    :global(.app:not(.mobile)) .guide .power { width: 64px; height: 64px; }
    :global(.app:not(.mobile)) .guide .power :global(.i) { width: 26px; height: 26px; }
  }
  .togs { display: flex; gap: 10px; flex-wrap: wrap; }

  /* Phone: the connection is the page. Big centred button, status beneath it. */
  :global(.app.mobile) .hero { padding: 28px 16px 20px; }
  :global(.app.mobile) .hero-top { flex-direction: column; align-items: center; text-align: center; gap: 16px; }
  :global(.app.mobile) .hero-text { align-items: center; width: 100%; }
  :global(.app.mobile) .power { width: 148px; height: 148px; }
  :global(.app.mobile) .power :global(.i) { width: 52px; height: 52px; }
  :global(.app.mobile) .state { font-size: 30px; }
  :global(.app.mobile) .route, :global(.app.mobile) .hero-meta { justify-content: center; }
  :global(.app.mobile) .hero-meta .chip { text-align: start; }
  :global(.app.mobile) .hero-controls { grid-template-columns: minmax(0, 1fr); }
  :global(.app.mobile) .hero-controls .seg { display: flex; }
  :global(.app.mobile) .hero-controls .seg button { flex: 1; }
  .tog { display: flex; align-items: center; gap: 10px; padding: 6px 8px 6px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--win); }
  .tog :global(.i) { color: var(--muted); }
  .tog span { font-size: 13px; font-weight: 500; }
  .tog small { display: block; color: var(--faint); font-size: 11px; font-weight: 400; }
  .exit { display: flex; flex-direction: column; gap: 14px; }
  .quality { grid-column: 1 / -1; display: flex; flex-direction: column; gap: 12px; }
  .quality .card-h { margin: 0; }
  .qgrid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 16px; }
  @container (max-width: 700px) { .qgrid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .qgrid > div { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .qgrid .v { font: 500 20px var(--f-mono); letter-spacing: -.02em; }
  .qgrid .v.warn { color: var(--warn); }
  .qgrid .d { font-size: 12px; color: var(--muted); }
  .qhint { margin: 0; font-size: 13px; }
  .ip { font: 500 22px var(--f-mono); letter-spacing: -.01em; }
  .why { color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
  .foot { display: flex; gap: 8px; flex-wrap: wrap; }
  .legend { display: flex; gap: 16px; flex-wrap: wrap; margin-inline-start: auto; }
  .leg { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--muted); }
  .leg i { width: 10px; height: 3px; border-radius: 2px; display: block; }
  .leg b { font: 500 14px var(--f-mono); color: var(--fg); }
  .qs { display: flex; flex-direction: column; gap: 4px; }
  .qs button { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 1px solid transparent; border-radius: 8px; background: none; text-align: start; min-width: 0; }
  @media (hover: hover) { .qs button:hover { background: var(--sunk); } }
  .qs button[aria-pressed='true'] { border-color: var(--faint); background: var(--accent-soft); }
  .qs .nm { font-weight: 500; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .qs .pr { color: var(--muted); font-size: 12px; font-weight: 400; }
  .more { margin-top: 8px; }
  .tile { display: flex; flex-direction: column; gap: 4px; }
  .tile .v { font: 500 22px var(--f-mono); letter-spacing: -.02em; }
  .tile .d { font-size: 12px; color: var(--muted); }
  .tile .pair { display: flex; gap: 24px; flex-wrap: wrap; }
</style>
