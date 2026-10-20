<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type Learned, type Preset, type Rule, type RuleKind } from '../lib/api';
  import { fmtAgo } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  let PRESETS = $derived<{ id: Preset; title: string; desc: string; needs?: 'runetfreedom' }[]>([
    { id: 'proxy-blocked-ru', title: t('Only blocked sites through the proxy'), desc: t('Sites blocked in Russia use the proxy; everything else goes direct. Uses the Russia routing data.'), needs: 'runetfreedom' },
    { id: 'direct-ru', title: t('Direct for Russian sites'), desc: t('.ru, .рф, .su, Russian services and IP ranges skip the proxy.') },
    { id: 'block-ads', title: t('Block ads and trackers'), desc: t('Rejects the category-ads-all list.') },
    { id: 'direct-cn', title: t('Direct for Chinese sites'), desc: t('geosite:cn and Chinese IP ranges go direct.') },
    { id: 'direct-ir', title: t('Direct for Iranian sites'), desc: t('.ir domains and Iranian IP ranges go direct.') },
  ]);
  let KINDS = $derived<[RuleKind, string, string][]>([
    ['domain-suffix', t('Domain and subdomains'), 'example.org'],
    ['domain', t('Exact domain'), 'api.github.com'],
    ['domain-keyword', t('Domain contains'), 'google'],
    ['geosite', t('Geosite list'), 'youtube'],
    ['geoip', t('Country (GeoIP)'), 'DE'],
    ['ip-cidr', t('IP range'), '203.0.113.0/24'],
    ['process-name', t('App (process name)'), 'steam.exe'],
  ]);
  let KIND_LABEL = $derived(Object.fromEntries(KINDS.map(([k, l]) => [k, l])));

  let kind = $state<RuleKind>('domain-suffix');
  let value = $state('');
  let target = $state('Proxy');

  let s = $derived(app.settings);
  let rules = $derived(s?.core.custom_rules ?? []);
  let targets = $derived(['Proxy', 'DIRECT', 'REJECT', ...app.proxies.groups.map((g) => g.name).filter((g) => !['Proxy', 'GLOBAL'].includes(g))]);
  let isClash = $derived(app.activeProfile?.format === 'clash');

  function togglePreset(p: Preset) {
    void app.saveSettings((x) => {
      x.core.presets = x.core.presets.includes(p) ? x.core.presets.filter((y) => y !== p) : [...x.core.presets, p];
    });
  }

  function add() {
    const v = value.trim();
    if (!v) return;
    const rule: Rule = { kind, value: v, target };
    void app.saveSettings((x) => (x.core.custom_rules = [...x.core.custom_rules, rule]), t('Rule added'));
    value = '';
  }

  function move(i: number, d: -1 | 1) {
    void app.saveSettings((x) => {
      const r = [...x.core.custom_rules];
      const j = i + d;
      if (j < 0 || j >= r.length) return;
      [r[i], r[j]] = [r[j], r[i]];
      x.core.custom_rules = r;
    });
  }

  // Drag to reorder. The rows stay where they are in the page while dragging: the
  // dragged one follows the pointer and the others slide aside with a transform, and
  // the order only changes on release. (Moving the row element itself would make the
  // browser drop the pointer capture, and the release would never arrive.) Pointer
  // events rather than HTML drag-and-drop, which the desktop webview keeps for files.
  let tbody = $state<HTMLTableSectionElement>();
  let drag = $state<{ from: number; to: number; dy: number; height: number } | null>(null);
  // Positions are measured from the top of the table body, which itself never moves
  // relative to its rows, so scrolling the page mid-drag doesn't throw the target off.
  let startY = 0;
  let lastClientY = 0;
  let mids: number[] = [];
  const tableTop = () => tbody?.getBoundingClientRect().top ?? 0;

  function grab(e: PointerEvent, i: number) {
    if (e.button !== 0 || !tbody) return;
    e.preventDefault();
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    const top = tableTop();
    const rects = [...tbody.rows].map((r) => r.getBoundingClientRect());
    mids = rects.map((b) => b.top - top + b.height / 2);
    lastClientY = e.clientY;
    startY = e.clientY - top;
    drag = { from: i, to: i, dy: 0, height: rects[i].height };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', cancelDrag);
    window.addEventListener('blur', cancelDrag);
    window.addEventListener('keydown', onEscape);
    window.addEventListener('scroll', follow, true);
  }

  function onMove(e: PointerEvent) {
    if (!drag) return;
    // Released somewhere we couldn't hear about (outside the window): finish now.
    if ((e.buttons & 1) === 0) return onUp();
    lastClientY = e.clientY;
    follow();
  }

  /** Place the dragged row under the pointer; also called when the page scrolls. */
  function follow() {
    if (!drag) return;
    const y = lastClientY - tableTop();
    // The new position is the number of other rows whose middle is above the pointer.
    const to = mids.filter((m, j) => j !== drag!.from && m < y).length;
    drag = { ...drag, to, dy: y - startY };
  }

  function onUp() {
    const d = drag;
    endDrag();
    if (d && d.to !== d.from) {
      const next = [...rules];
      const [moved] = next.splice(d.from, 1);
      next.splice(d.to, 0, moved);
      void app.saveSettings((x) => (x.core.custom_rules = next), t('Rule order saved'));
    }
  }

  function onEscape(e: KeyboardEvent) {
    if (e.key === 'Escape') cancelDrag();
  }

  function cancelDrag() {
    endDrag();
  }

  function endDrag() {
    drag = null;
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', cancelDrag);
    window.removeEventListener('blur', cancelDrag);
    window.removeEventListener('keydown', onEscape);
    window.removeEventListener('scroll', follow, true);
  }

  // Leaving the page mid-drag must not leave window listeners behind.
  onDestroy(endDrag);

  /** How far row `j` is shifted while a drag is in progress. */
  function shift(j: number): number {
    if (!drag) return 0;
    const { from, to, dy, height } = drag;
    if (j === from) return dy;
    if (from < to && j > from && j <= to) return -height;
    if (to < from && j >= to && j < from) return height;
    return 0;
  }

  function gripKey(e: KeyboardEvent, k: number) {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      move(k, e.key === 'ArrowUp' ? -1 : 1);
    }
  }

  // Smart routing
  let learned = $state<Learned[]>([]);
  const loadLearned = async () => { learned = (await app.run(() => api.learnedSites())) ?? learned; };
  $effect(() => { void app.learnedVersion; void loadLearned(); });
  let REASON = $derived<Record<string, string>>({
    timeout: t('no answer'), reset: t('connection reset'), tls: t('secure connection broken'), dns: t('name not resolved'),
    'connect-failed': t('could not connect'), refused: t('access refused'),
  });
  async function forget(domain: string) {
    await app.run(() => api.forgetLearned(domain), t('{domain} forgotten', { domain }));
    await loadLearned();
  }
  async function keep(domain: string) {
    const saved = await app.run(() => api.keepLearned(domain), t('{domain} moved to your rules', { domain }));
    if (saved) app.settings = saved;
    await loadLearned();
  }
  async function clearAll() {
    await app.run(() => api.clearLearned(), t('Learned sites cleared'));
    await loadLearned();
  }

  const remove = (i: number) => app.saveSettings((x) => (x.core.custom_rules = x.core.custom_rules.filter((_, k) => k !== i)));
  const polClass = (t: string) => (t === 'DIRECT' ? 'direct' : t === 'REJECT' ? 'reject' : 'proxy');
</script>

<div class="page-head">
  <h1>{t('Rules')}</h1><span class="sub">{t('The first matching rule decides where traffic goes')}</span>
</div>

{#if s}
  <div class="eyebrow">{t('Routing presets')}</div>
  <div class="presets">
    {#each PRESETS as p (p.id)}
      {@const unavailable = p.needs === 'runetfreedom' && s.core.geodata.kind === 'default' && !s.core.presets.includes(p.id)}
      <div class="card preset">
        <div><div class="t">{p.title}</div><div class="d">{p.desc}</div>
          {#if unavailable}<button class="link" onclick={() => (app.view = 'settings')}>{t('Switch routing data to Russia in Settings')}</button>{/if}</div>
        <button class="switch" role="switch" aria-label={p.title} aria-checked={s.core.presets.includes(p.id)} disabled={unavailable} onclick={() => togglePreset(p.id)}></button>
      </div>
    {/each}
  </div>

  <section class="card">
    <div class="card-h"><h2>{t('Your rules')}</h2><span class="aside">{t('checked before presets')}</span></div>
    <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <select class="input" bind:value={kind} aria-label={t('Match type')}>
        {#each KINDS as [k, l] (k)}<option value={k}>{l}</option>{/each}
      </select>
      <input class="input mono" bind:value={value} placeholder={KINDS.find((k) => k[0] === kind)?.[2]} aria-label={t('Value')} />
      <select class="input" bind:value={target} aria-label={t('Send to')}>
        {#each targets as x (x)}<option value={x}>{x === 'Proxy' ? t('Proxy (main group)') : x}</option>{/each}
      </select>
      <button class="btn primary" type="submit" disabled={!value.trim()}><Icon name="plus" />{t('Add rule')}</button>
    </form>
    {#if rules.length}
      <div class="twrap list">
        <table>
          <thead><tr><th></th><th>#</th><th>{t('Match')}</th><th>{t('Value')}</th><th>{t('Send to')}</th><th></th></tr></thead>
          <tbody bind:this={tbody}>
            {#each rules as r, i (i)}
              <tr class:dragging={drag?.from === i} class:moving={drag !== null && drag.from !== i} style:transform={drag ? `translateY(${shift(i)}px)` : null}>
                <td class="grip-cell">
                  <button class="grip" aria-label={t('Move rule {n}: drag, or use the arrow keys', { n: i + 1 })}
                    onpointerdown={(e) => grab(e, i)} onkeydown={(e) => gripKey(e, i)}><Icon name="grip" /></button>
                </td>
                <td class="num muted">{i + 1}</td>
                <td><span class="rt">{KIND_LABEL[r.kind] ?? r.kind}</span></td>
                <td class="mono small selectable">{r.value}</td>
                <td><span class="pol {polClass(r.target)}">{r.target}</span></td>
                <td class="r"><button class="x" onclick={() => remove(i)} aria-label={t('Delete rule {n}', { n: i + 1 })}><Icon name="x" /></button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if rules.length > 1}<p class="hint">{t('Drag')} <Icon name="grip" class="inline-grip" /> {t('to change the order, or focus it and press ↑ or ↓. Rules higher up win.')}</p>{/if}
    {:else}
      <p class="hint">{t('No rules of your own yet. Add one to send a site or app through the proxy, direct, or block it.')}</p>
    {/if}
    <div class="row-sw apply">
      <div><div class="t">{t('Apply to imported Clash profiles')}</div><div class="d">{isClash ? t('Put your rules and presets in front of a Clash profile’s own rules (the active profile is one).') : t('Put your rules and presets in front of a Clash profile’s own rules.')}</div></div>
      <button class="switch" role="switch" aria-label={t('Apply to imported Clash profiles')} aria-checked={s.core.apply_rules_to_clash_profiles}
        onclick={() => app.saveSettings((x) => (x.core.apply_rules_to_clash_profiles = !x.core.apply_rules_to_clash_profiles))}></button>
    </div>
  </section>

  <section class="card smart">
    <div class="card-h"><h2>{t('Smart routing')}</h2><span class="chip">beta</span>
      <button class="switch aside-sw" role="switch" aria-label={t('Smart routing')} aria-checked={s.core.smart_routing}
        onclick={() => app.saveSettings((x) => (x.core.smart_routing = !x.core.smart_routing))}></button></div>
    <p class="hint">{t('Kova notices sites that fail on their usual path, checks them directly and through the proxy, and sends each one the way that works: blocked sites through the proxy, sites that refuse the proxy direct. With TUN it also learns apps that connect by IP address. Learned sites are kept for 30 days.')}</p>
    {#if s.core.smart_routing && s.core.mode !== 'rule'}
      <p class="hint warn-text">{t('Learning is paused: it works in Rule mode only.')}</p>
    {:else if s.core.smart_routing && s.core.tun.enabled && !s.core.sniff && !s.core.dns.fake_ip}
      <p class="hint warn-text">{t('In TUN mode Kova needs site names: turn on Sniff domains or Fake-IP in Settings, otherwise it can only learn IP addresses.')}</p>
    {:else if s.core.smart_routing && isClash && !s.core.apply_rules_to_clash_profiles}
      <p class="hint warn-text">{t('Turn on “Apply to imported Clash profiles” above: the active profile is a Clash profile.')}</p>
    {/if}
    {#if learned.length}
      <div class="twrap list">
        <table>
          <thead><tr><th>{t('Site')}</th><th>{t('Now goes')}</th><th>{t('Why')}</th><th>{t('Learned')}</th><th></th></tr></thead>
          <tbody>
            {#each learned as l (l.domain)}
              <tr>
                <td class="mono small selectable">{l.domain}</td>
                <td><span class="pol {l.route === 'proxy' ? 'proxy' : 'direct'}">{l.route === 'proxy' ? t('Through the proxy') : t('Direct')}</span></td>
                <td class="muted">{REASON[l.reason] ?? l.reason}</td>
                <td class="muted">{fmtAgo(l.learned_at)}</td>
                <td class="r"><div class="ops">
                  <button class="btn ghost small-btn" onclick={() => keep(l.domain)} title={t('Make it one of your rules, kept for good')}>{t('Keep')}</button>
                  <button class="x" onclick={() => forget(l.domain)} aria-label={t('Forget {domain}', { domain: l.domain })}><Icon name="x" /></button>
                </div></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="actions gap"><button class="btn ghost danger" onclick={clearAll}>{t('Forget all')}</button></div>
    {:else if s.core.smart_routing}
      <p class="hint">{t('Nothing learned yet. A site appears here when Kova finds it works only the other way.')}</p>
    {/if}
  </section>

  <p class="hint order">{t('Order: your rules → presets → local network goes direct → everything else uses')} <b>{s.core.presets.includes('proxy-blocked-ru') ? 'DIRECT' : 'Proxy'}</b>. {t('Global and Direct modes skip rules entirely.')}</p>
{/if}

<style>
  .presets { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px; margin: 8px 0 16px; }
  .preset { display: flex; gap: 12px; align-items: flex-start; }
  .preset .t { font-weight: 600; font-size: 13.5px; }
  .preset .d { font-size: 12px; color: var(--muted); margin-top: 2px; }
  .preset .switch { margin-left: auto; margin-top: 2px; }
  .link { border: 0; background: none; padding: 0; margin-top: 6px; color: var(--fg); font-size: 12px; text-decoration: underline; text-underline-offset: 2px; }
  .add { display: grid; grid-template-columns: 200px minmax(0, 1fr) 200px auto; gap: 8px; }
  @container (max-width: 760px) { .add { grid-template-columns: 1fr 1fr; } }
  .list { margin-top: 12px; }
  .small { font-size: 12px; }
  .grip-cell { width: 1%; padding-right: 0; }
  .grip { border: 0; background: none; color: var(--faint); width: 22px; height: 26px; border-radius: 5px; display: grid; place-items: center; cursor: grab; touch-action: none; }
  .grip:hover { color: var(--fg); background: var(--sunk); }
  .grip:active { cursor: grabbing; }
  .grip :global(.i) { width: 16px; height: 16px; stroke-width: 2.8; }
  tr.dragging td { background: var(--accent-soft); }
  tr.dragging { box-shadow: 0 6px 20px rgb(0 0 0 / .35); position: relative; z-index: 2; }
  tr.moving { transition: transform .14s ease; }
  tbody:has(tr.dragging) { user-select: none; cursor: grabbing; }
  .hint :global(.inline-grip) { width: 13px; height: 13px; stroke-width: 3; vertical-align: -2px; }
  .apply { margin-top: 12px; border-top: 1px solid var(--line); padding-top: 12px; }
  .order { margin-top: 14px; }
  .smart { margin-top: 12px; }
  .smart .card-h { gap: 8px; }
  .aside-sw { margin-left: auto; }
  .ops { display: flex; gap: 4px; justify-content: flex-end; align-items: center; }
  .small-btn { height: 26px; padding: 0 9px; font-size: 12px; }
  .gap { margin-top: 10px; }
  .order b { color: var(--fg); font-weight: 600; }
</style>
