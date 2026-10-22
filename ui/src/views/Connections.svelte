<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type Rule, type Connection } from '../lib/api';
  import { displayName, fmtBytes, fmtDur, fmtSpeed } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  interface Row extends Connection { dl: number; ul: number; started: number }

  let rows = $state<Row[]>([]);
  let q = $state('');
  let filter = $state<'all' | 'proxy' | 'direct'>('all');
  let paused = $state(false);
  let prev = new Map<string, { up: number; down: number }>();
  let now = $state(Date.now());

  async function poll() {
    now = Date.now();
    if (paused || !app.status.running) { if (!app.status.running) rows = []; return; }
    try {
      const snap = await api.getConnections();
      const next = new Map<string, { up: number; down: number }>();
      rows = snap.connections.map((c) => {
        const p = prev.get(c.id);
        next.set(c.id, { up: c.upload, down: c.download });
        return { ...c, dl: p ? Math.max(0, c.download - p.down) : 0, ul: p ? Math.max(0, c.upload - p.up) : 0, started: Date.parse(c.start) || Date.now() };
      });
      prev = next;
    } catch { /* core restarting */ }
  }
  void poll();
  const timer = setInterval(poll, 1000);
  onDestroy(() => clearInterval(timer));

  const isDirect = (r: Row) => r.chains.length === 0 || r.chains[r.chains.length - 1] === 'DIRECT';
  let shown = $derived(rows
    .filter((r) => filter === 'all' || (filter === 'direct') === isDirect(r))
    .filter((r) => !q || [r.host, r.process, r.rule, r.chains.join(' ')].join(' ').toLowerCase().includes(q.toLowerCase()))
    .sort((a, b) => b.dl + b.ul - (a.dl + a.ul) || b.started - a.started));

  async function close(r: Row) {
    await app.run(() => api.closeConnection(r.id));
    rows = rows.filter((x) => x.id !== r.id);
  }
  /** Route this connection's site the other way from now on, with a rule above all others. */
  async function flip(r: Row) {
    const toDirect = !isDirect(r);
    const ip = /^[\d.]+$/.test(r.host) || r.host.includes(':');
    const value = ip ? `${r.host}/${r.host.includes(':') ? 128 : 32}` : r.host.replace(/^www\./, '');
    const rule: Rule = { kind: ip ? 'ip-cidr' : 'domain-suffix', value, target: toDirect ? 'DIRECT' : 'Proxy' };
    await app.saveSettings((x) => {
      x.core.custom_rules = [rule, ...x.core.custom_rules.filter((y) => !(y.kind === rule.kind && y.value === rule.value))];
    }, toDirect ? t('{host} now always goes direct', { host: value }) : t('{host} now always goes through the proxy', { host: value }));
    await app.run(() => api.closeConnection(r.id));
  }

  async function closeAll() {
    const n = rows.length;
    await app.run(() => api.closeAllConnections(), tn(n, 'Closed {n} connection', 'Closed {n} connections'));
    rows = [];
  }
</script>

<div class="page-head">
  <h1>{t('Connections')}</h1>
  <span class="sub">{app.status.running ? t('{shown} of {total} open', { shown: shown.length, total: rows.length }) : t('Not connected')}</span>
  <div class="tools">
    <label class="search"><Icon name="search" /><input class="input" placeholder={t('Filter by host, app or rule')} bind:value={q} aria-label={t('Filter connections')} /></label>
    <div class="seg" role="group" aria-label={t('Show')}>
      {#each [['all', t('All')], ['proxy', t('Proxied')], ['direct', t('Direct')]] as [v, l] (v)}
        <button aria-pressed={filter === v} onclick={() => (filter = v as typeof filter)}>{l}</button>
      {/each}
    </div>
    <button class="btn" onclick={() => (paused = !paused)}><Icon name={paused ? 'play' : 'pause'} />{paused ? t('Resume') : t('Pause')}</button>
    <button class="btn danger" onclick={closeAll} disabled={!rows.length}><Icon name="x" />{t('Close all')}</button>
  </div>
</div>

{#if app.status.running && app.status.core === 'xray'}
  <div class="empty"><b>{t('Live connections need Mihomo')}</b><span>{t('This profile runs on Xray, which doesn’t report individual connections. Switch the profile’s core to Automatic to see them.')}</span></div>
{:else}
  <div class="twrap">
    <table>
      <thead><tr><th>{t('Host')}</th><th>{t('App')}</th><th>{t('Rule')}</th><th>{t('Route')}</th><th class="r">↓</th><th class="r">↑</th><th class="r">{t('Total')}</th><th class="r">{t('Time')}</th><th></th></tr></thead>
      <tbody>
        {#each shown as r (r.id)}
          <tr>
            <td><span class="host selectable">{r.host}</span><span class="muted">:{r.port}</span> <span class="chip mono">{r.network}</span></td>
            <td class="muted">{r.process || '—'}</td>
            <td><span class="rt">{r.rule}</span></td>
            <td><span class="pol" class:direct={isDirect(r)} class:proxy={!isDirect(r)}>{r.chains.map(displayName).join(' › ')}</span></td>
            <td class="r num dn">{r.dl > 0 ? fmtSpeed(r.dl) : '—'}</td>
            <td class="r num up">{r.ul > 0 ? fmtSpeed(r.ul) : '—'}</td>
            <td class="r num">{fmtBytes(r.upload + r.download)}</td>
            <td class="r num">{fmtDur(now - r.started)}</td>
            <td class="acts">
              {#if r.host}<button class="x" onclick={() => flip(r)} title={isDirect(r) ? t('Always send {host} through the proxy', { host: r.host }) : t('Always send {host} direct', { host: r.host })}
                aria-label={isDirect(r) ? t('Always send {host} through the proxy', { host: r.host }) : t('Always send {host} direct', { host: r.host })}><Icon name="connections" /></button>{/if}
              <button class="x" onclick={() => close(r)} aria-label={t('Close connection to {host}', { host: r.host })}><Icon name="x" /></button></td>
          </tr>
        {:else}
          <tr><td colspan="9" class="none">{app.status.running ? t('No open connections match this filter.') : t('Connect to see live connections.')}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .host { font-weight: 500; }
  .none { color: var(--muted); padding: 22px 12px; }
  .twrap { max-height: calc(100vh - 190px); }
</style>
