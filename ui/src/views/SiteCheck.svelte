<script lang="ts">
  import CheckList from '../components/CheckList.svelte';
  import type { Check } from '../lib/api';
  import { api, errText, type CheckSite, type Probe, type SiteResult } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  let defaults = $state<CheckSite[]>([]);
  api.defaultCheckSites().then((s) => (defaults = s)).catch(() => {});

  const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return url; } };
  const OWN = 'Your sites';
  let custom = $derived<CheckSite[]>((app.settings?.check_sites ?? []).map((url) => ({ name: hostOf(url), url, group: OWN })));
  let sites = $derived([...defaults, ...custom]);
  let groups = $derived([...new Set(sites.map((s) => s.group))]);

  // The built-in list comes from the engine in English.
  let GROUP = $derived<Record<string, string>>({
    Baseline: t('Baseline'), Video: t('Video'), Social: t('Social'), Messengers: t('Messengers'), AI: t('AI'), Other: t('Other'), [OWN]: t('Your sites'),
  });
  let SITE = $derived<Record<string, string>>({
    'Google (connectivity check)': t('Google (connectivity check)'), 'YouTube video servers': t('YouTube video servers'),
  });

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
    try { new URL(url); } catch { app.notify(t('That doesn’t look like a web address')); return; }
    if (sites.some((s) => s.url === url)) { app.notify(t('That site is already in the list')); return; }
    void app.saveSettings((x) => (x.check_sites = [...x.check_sites, url]));
    newUrl = '';
    void checkOne(url);
  }

  const removeSite = (url: string) => app.saveSettings((x) => (x.check_sites = x.check_sites.filter((u) => u !== url)));

  const works = (p?: Probe | null) => !!p && (p.outcome === 'ok' || p.outcome === 'http-error');

  /** The one-word answer the user is looking for. */
  function verdict(r?: SiteResult): { text: string; cls: string } | null {
    if (!r) return null;
    if (!r.via_kova) return works(r.direct) ? { text: t('Reachable'), cls: '' } : { text: r.direct.outcome === 'refused' ? t('Region-locked') : t('Blocked'), cls: r.direct.outcome === 'refused' ? 'warn' : 'bad' };
    if (works(r.via_kova)) return works(r.direct) ? { text: t('Reachable'), cls: '' } : { text: t('Unblocked by Kova'), cls: 'ok' };
    if (r.via_kova.outcome === 'refused') return { text: t('Region-locked'), cls: 'warn' };
    return { text: t('Unreachable'), cls: 'bad' };
  }

  /** The failure description, from the outcome where the engine's text is fixed. */
  function detail(p: Probe): string {
    switch (p.outcome) {
      case 'timeout': return t('No answer within 10 seconds');
      case 'dns': return t('The site’s name could not be resolved');
      case 'tls': return t('The secure connection was broken, which is typical of DPI blocking');
      case 'reset': return t('The connection was reset, which is typical of blocking');
      case 'refused': return t('The site refused access (HTTP {status}); often a region lock', { status: p.status ?? '' });
      case 'http-error': return t('The site answered with HTTP {status}', { status: p.status ?? '' });
      case 'ok': return p.detail ? t('Answered with a bot check, so the site is reachable') : '';
      default: return p.detail ?? '';
    }
  }

  let checked = $derived(Object.values(results));
  let summary = $derived({
    reachable: checked.filter((r) => works(r.via_kova ?? r.direct)).length,
    unblocked: checked.filter((r) => r.via_kova && works(r.via_kova) && !works(r.direct)).length,
  });
  let connected = $derived(app.status.running);

  let privacy = $state<Check[] | null>(null);
  let privacyBusy = $state(false);
  async function checkPrivacy() {
    privacyBusy = true;
    privacy = (await app.run(() => api.privacyCheck())) ?? privacy;
    privacyBusy = false;
  }

  let speed = $state<{ through_kova: number | null; direct: number | null } | null>(null);
  let speedBusy = $state(false);
  async function testSpeed() {
    speedBusy = true;
    speed = (await app.run(() => api.speedTest())) ?? speed;
    speedBusy = false;
  }
  const mbps = (v: number | null) => (v == null ? '—' : t('{n} Mbit/s', { n: v >= 100 ? Math.round(v) : v.toFixed(1) }));
</script>

<div class="page-head">
  <h1>{t('Site check')}</h1>
  <span class="sub">{t('Can these sites be reached, with Kova and without it?')}</span>
  <div class="tools">
    <form class="add" onsubmit={(e) => { e.preventDefault(); addSite(); }}>
      <input class="input mono" bind:value={newUrl} placeholder="example.com" aria-label={t('Site to add')} />
      <button class="btn" type="submit" disabled={!newUrl.trim()}><Icon name="plus" />{t('Add')}</button>
    </form>
    <button class="btn primary" class:busy={running} onclick={checkAll} disabled={running || !sites.length}>
      <Icon name={running ? 'refresh' : 'bolt'} />{running ? t('Checking…') : t('Check all')}</button>
  </div>
</div>

{#if !connected}
  <p class="note"><Icon name="alert" />{t('Kova isn’t connected, so only direct access is checked. Connect to compare.')}</p>
{:else if app.status.tun_active}
  <p class="note"><Icon name="alert" />{t('TUN is on, so “Without Kova” also goes through Kova and shows the same result.')}</p>
{/if}

<div class="tests">
  <section class="card">
    <div class="card-h"><h2>{t('Privacy check')}</h2>
      <button class="btn aside-btn" class:busy={privacyBusy} disabled={privacyBusy} onclick={checkPrivacy}><Icon name="shield" />{privacy ? t('Check again') : t('Check')}</button></div>
    {#if privacy}
      <CheckList checks={privacy} onfixed={() => setTimeout(checkPrivacy, 1500)} />
    {:else}
      <p class="muted small">{t('What sites can see: your address with Kova and without it, how site names are looked up, and what may go around Kova.')}</p>
    {/if}
  </section>
  <section class="card">
    <div class="card-h"><h2>{t('Speed test')}</h2>
      <button class="btn aside-btn" class:busy={speedBusy} disabled={speedBusy} onclick={testSpeed}><Icon name="down" />{speedBusy ? t('Measuring…') : speed ? t('Test again') : t('Test')}</button></div>
    {#if speed}
      <dl class="speeds">
        <dt>{t('Through Kova')}</dt><dd class="num">{connected ? mbps(speed.through_kova) : '—'}</dd>
        <dt>{t('Without Kova')}</dt><dd class="num">{mbps(speed.direct)}</dd>
      </dl>
      {#if app.status.tun_active}<p class="muted small">{t('TUN is on, so “Without Kova” also goes through Kova.')}</p>{/if}
    {:else}
      <p class="muted small">{t('Downloads a test file from Cloudflare for a few seconds, through Kova and without it.')}</p>
    {/if}
  </section>
</div>

{#if checked.length}
  <div class="summary">
    <span><b>{summary.reachable}</b> {connected ? t('of {n} reachable through Kova', { n: checked.length }) : t('of {n} reachable', { n: checked.length })}</span>
    {#if connected}<span><b>{summary.unblocked}</b> {t('only reachable with Kova')}</span>{/if}
  </div>
{/if}

{#each groups as g (g)}
  <div class="eyebrow group">{GROUP[g] ?? g}</div>
  <div class="twrap">
    <table>
      <thead><tr><th>{t('Site')}</th><th>{t('Through Kova')}</th><th>{t('Without Kova')}</th><th>{t('Result')}</th><th></th></tr></thead>
      <tbody>
        {#each sites.filter((s) => s.group === g) as s (s.url)}
          {@const r = results[s.url]}
          {@const v = verdict(r)}
          <tr>
            <td><div class="name">{SITE[s.name] ?? s.name}</div><div class="url mono">{hostOf(s.url)}</div></td>
            <td>{#if pending.has(s.url)}<span class="lat testing">…</span>{:else if r}{@render probe(r.via_kova)}{:else}<span class="muted">—</span>{/if}</td>
            <td>{#if pending.has(s.url)}<span class="lat testing">…</span>{:else if r}{@render probe(r.direct)}{:else}<span class="muted">—</span>{/if}</td>
            <td>{#if v}<span class="chip {v.cls}">{v.text}</span>{:else if failed[s.url]}<span class="fail">{failed[s.url]}</span>{/if}</td>
            <td class="r"><div class="ops">
              <button class="x" onclick={() => checkOne(s.url)} disabled={pending.has(s.url)} aria-label={t('Check {name} again', { name: s.name })}><Icon name="refresh" /></button>
              {#if g === OWN}<button class="x" onclick={() => removeSite(s.url)} aria-label={t('Remove {name}', { name: s.name })}><Icon name="x" /></button>{/if}
            </div></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/each}

{#snippet probe(p: Probe | null)}
  {#if !p}
    <span class="muted">{t('not connected')}</span>
  {:else if p.outcome === 'ok'}
    <span class="lat ok" title={detail(p)}>{t('{n} ms', { n: p.ms ?? 0 })}</span>
  {:else if p.outcome === 'http-error'}
    <span class="lat warn" title={detail(p)}>HTTP {p.status}</span>
  {:else}
    <span class="fail" title={p.detail ?? ''}>{detail(p)}</span>
  {/if}
{/snippet}

<style>
  .tests { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 14px; margin-bottom: 18px; align-items: start; }
  .aside-btn { margin-inline-start: auto; }
  .small { font-size: 12.5px; margin: 0; }
  .speeds { display: grid; grid-template-columns: auto 1fr; gap: 6px 16px; margin: 0 0 6px; }
  .speeds dt { color: var(--muted); }
  .speeds dd { margin: 0; font-weight: 600; }
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
