<script lang="ts">
  import { api, errText, type CheckSite, type Probe, type SiteResult } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  let defaults = $state<CheckSite[]>([]);
  api.defaultCheckSites().then((s) => (defaults = s)).catch(() => {});

  const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return url; } };
  let custom = $derived<CheckSite[]>((app.settings?.check_sites ?? []).map((url) => ({ name: hostOf(url), url, group: 'Your sites' })));
  let sites = $derived([...defaults, ...custom]);
  let groups = $derived([...new Set(sites.map((s) => s.group))]);

  let results = $state<Record<string, SiteResult>>({});
  let failed = $state<Record<string, string>>({});
  let pending = $state<Set<string>>(new Set());
  let running = $state(false);
  let newUrl = $state('');

  async function checkAll() {
    running = true;
    results = {};
    failed = {};
    const queue = [...sites];
    pending = new Set(queue.map((s) => s.url));
    // A few at a time: enough to be quick, few enough not to look like a flood.
    const worker = async () => {
      for (let s = queue.shift(); s; s = queue.shift()) {
        // Await first, then merge: spreading `results` before the await would drop
        // whatever other workers stored meanwhile.
        try {
          const r = await api.checkSite(s.url);
          results = { ...results, [s.url]: r };
        } catch (e) {
          failed = { ...failed, [s.url]: errText(e) };
        }
        const p = new Set(pending); p.delete(s.url); pending = p;
      }
    };
    await Promise.all(Array.from({ length: 6 }, worker));
    running = false;
  }

  async function checkOne(url: string) {
    pending = new Set([...pending, url]);
    try {
      const r = await api.checkSite(url);
      results = { ...results, [url]: r };
    } catch (e) {
      failed = { ...failed, [url]: errText(e) };
    }
    const p = new Set(pending); p.delete(url); pending = p;
  }

  function addSite() {
    let url = newUrl.trim();
    if (!url) return;
    if (!/^https?:\/\//.test(url)) url = `https://${url}`;
    try { new URL(url); } catch { app.notify('That doesn’t look like a web address'); return; }
    if (sites.some((s) => s.url === url)) { app.notify('That site is already in the list'); return; }
    void app.saveSettings((x) => (x.check_sites = [...x.check_sites, url]));
    newUrl = '';
    void checkOne(url);
  }

  const removeSite = (url: string) => app.saveSettings((x) => (x.check_sites = x.check_sites.filter((u) => u !== url)));

  const works = (p?: Probe | null) => !!p && (p.outcome === 'ok' || p.outcome === 'http-error');

  /** The one-word answer the user is looking for. */
  function verdict(r?: SiteResult): { text: string; cls: string } | null {
    if (!r) return null;
    if (!r.via_kova) return works(r.direct) ? { text: 'Open', cls: '' } : { text: r.direct.outcome === 'refused' ? 'Region-locked' : 'Blocked', cls: r.direct.outcome === 'refused' ? 'warn' : 'bad' };
    if (works(r.via_kova)) return works(r.direct) ? { text: 'Open', cls: '' } : { text: 'Unblocked by Kova', cls: 'ok' };
    if (r.via_kova.outcome === 'refused') return { text: 'Region-locked', cls: 'warn' };
    return { text: 'Unreachable', cls: 'bad' };
  }

  let checked = $derived(Object.values(results));
  let summary = $derived({
    reachable: checked.filter((r) => works(r.via_kova ?? r.direct)).length,
    unblocked: checked.filter((r) => r.via_kova && works(r.via_kova) && !works(r.direct)).length,
  });
  let connected = $derived(app.status.running);
</script>

<div class="page-head">
  <h1>Site check</h1>
  <span class="sub">Can these sites be reached, with Kova and without it?</span>
  <div class="tools">
    <form class="add" onsubmit={(e) => { e.preventDefault(); addSite(); }}>
      <input class="input mono" bind:value={newUrl} placeholder="example.com" aria-label="Site to add" />
      <button class="btn" type="submit" disabled={!newUrl.trim()}><Icon name="plus" />Add</button>
    </form>
    <button class="btn primary" class:busy={running} onclick={checkAll} disabled={running || !sites.length}>
      <Icon name={running ? 'refresh' : 'bolt'} />{running ? 'Checking…' : 'Check all'}</button>
  </div>
</div>

{#if !connected}
  <p class="note"><Icon name="alert" />Kova isn’t connected, so only direct access is checked. Connect to compare.</p>
{:else if app.status.tun_active}
  <p class="note"><Icon name="alert" />TUN is on, so “Without Kova” also goes through Kova and shows the same result.</p>
{/if}

{#if checked.length}
  <div class="summary">
    <span><b>{summary.reachable}</b> of {checked.length} reachable{connected ? ' through Kova' : ''}</span>
    {#if connected}<span><b>{summary.unblocked}</b> only reachable with Kova</span>{/if}
  </div>
{/if}

{#each groups as g (g)}
  <div class="eyebrow group">{g}</div>
  <div class="twrap">
    <table>
      <thead><tr><th>Site</th><th>Through Kova</th><th>Without Kova</th><th>Result</th><th></th></tr></thead>
      <tbody>
        {#each sites.filter((s) => s.group === g) as s (s.url)}
          {@const r = results[s.url]}
          {@const v = verdict(r)}
          <tr>
            <td><div class="name">{s.name}</div><div class="url mono">{hostOf(s.url)}</div></td>
            <td>{#if pending.has(s.url)}<span class="lat testing">…</span>{:else if r}{@render probe(r.via_kova)}{:else}<span class="muted">—</span>{/if}</td>
            <td>{#if pending.has(s.url)}<span class="lat testing">…</span>{:else if r}{@render probe(r.direct)}{:else}<span class="muted">—</span>{/if}</td>
            <td>{#if v}<span class="chip {v.cls}">{v.text}</span>{:else if failed[s.url]}<span class="fail">{failed[s.url]}</span>{/if}</td>
            <td class="r"><div class="ops">
              <button class="x" onclick={() => checkOne(s.url)} disabled={pending.has(s.url)} aria-label="Check {s.name} again"><Icon name="refresh" /></button>
              {#if g === 'Your sites'}<button class="x" onclick={() => removeSite(s.url)} aria-label="Remove {s.name}"><Icon name="x" /></button>{/if}
            </div></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/each}

{#snippet probe(p: Probe | null)}
  {#if !p}
    <span class="muted">not connected</span>
  {:else if p.outcome === 'ok'}
    <span class="lat ok">{p.ms} ms</span>
  {:else if p.outcome === 'http-error'}
    <span class="lat warn" title={p.detail ?? ''}>HTTP {p.status}</span>
  {:else}
    <span class="fail" title={p.detail ?? ''}>{p.detail}</span>
  {/if}
{/snippet}

<style>
  .add { display: flex; gap: 8px; }
  .add .input { width: 200px; }
  .note { display: flex; gap: 8px; align-items: center; color: var(--muted); font-size: 12.5px; margin: -6px 0 14px; }
  .note :global(.i) { width: 15px; height: 15px; color: var(--warn); }
  .summary { display: flex; gap: 20px; margin-bottom: 6px; color: var(--muted); font-size: 13px; }
  .summary b { color: var(--fg); font-family: var(--f-mono); font-weight: 500; }
  .group { margin: 16px 0 8px; }
  table { table-layout: fixed; }
  th:nth-child(1) { width: 24%; } th:nth-child(2), th:nth-child(3) { width: 28%; } th:nth-child(4) { width: 14%; } th:nth-child(5) { width: 6%; }
  td { white-space: normal; vertical-align: middle; }
  .name { font-weight: 500; }
  .url { font-size: 11.5px; color: var(--faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .fail { color: var(--bad); font-size: 12px; line-height: 1.35; display: block; }
  .chip.ok { background: var(--ok-soft); border-color: transparent; color: var(--ok); }
  .ops { display: flex; gap: 2px; justify-content: flex-end; }
  .x:disabled { opacity: .3; cursor: default; }
</style>
