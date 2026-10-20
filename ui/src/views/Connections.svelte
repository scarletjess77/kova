<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type Connection } from '../lib/api';
  import { displayName, fmtBytes, fmtDur, fmtSpeed } from '../lib/format';
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
  async function closeAll() {
    const n = rows.length;
    await app.run(() => api.closeAllConnections(), `Closed ${n} connections`);
    rows = [];
  }
</script>

<div class="page-head">
  <h1>Connections</h1>
  <span class="sub">{app.status.running ? `${shown.length} of ${rows.length} open` : 'Not connected'}</span>
  <div class="tools">
    <label class="search"><Icon name="search" /><input class="input" placeholder="Filter by host, app or rule" bind:value={q} aria-label="Filter connections" /></label>
    <div class="seg" role="group" aria-label="Show">
      {#each [['all', 'All'], ['proxy', 'Proxied'], ['direct', 'Direct']] as [v, l] (v)}
        <button aria-pressed={filter === v} onclick={() => (filter = v as typeof filter)}>{l}</button>
      {/each}
    </div>
    <button class="btn" onclick={() => (paused = !paused)}><Icon name={paused ? 'play' : 'pause'} />{paused ? 'Resume' : 'Pause'}</button>
    <button class="btn danger" onclick={closeAll} disabled={!rows.length}><Icon name="x" />Close all</button>
  </div>
</div>

{#if app.status.running && app.status.core === 'xray'}
  <div class="empty"><b>Live connections need Mihomo</b><span>This profile runs on Xray, which doesn’t report individual connections. Switch the profile’s core to Automatic to see them.</span></div>
{:else}
  <div class="twrap">
    <table>
      <thead><tr><th>Host</th><th>App</th><th>Rule</th><th>Route</th><th class="r">↓</th><th class="r">↑</th><th class="r">Total</th><th class="r">Time</th><th></th></tr></thead>
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
            <td><button class="x" onclick={() => close(r)} aria-label="Close connection to {r.host}"><Icon name="x" /></button></td>
          </tr>
        {:else}
          <tr><td colspan="9" class="none">{app.status.running ? 'No open connections match this filter.' : 'Connect to see live connections.'}</td></tr>
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
