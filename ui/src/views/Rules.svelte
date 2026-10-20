<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { Preset, Rule, RuleKind } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  const PRESETS: { id: Preset; title: string; desc: string; needs?: 'runetfreedom' }[] = [
    { id: 'proxy-blocked-ru', title: 'Only blocked sites through the proxy', desc: 'Sites blocked in Russia use the proxy; everything else goes direct. Uses the Russia routing data.', needs: 'runetfreedom' },
    { id: 'direct-ru', title: 'Direct for Russian sites', desc: '.ru, .рф, .su, Russian services and IP ranges skip the proxy.' },
    { id: 'block-ads', title: 'Block ads and trackers', desc: 'Rejects the category-ads-all list.' },
    { id: 'direct-cn', title: 'Direct for Chinese sites', desc: 'geosite:cn and Chinese IP ranges go direct.' },
    { id: 'direct-ir', title: 'Direct for Iranian sites', desc: '.ir domains and Iranian IP ranges go direct.' },
  ];
  const KINDS: [RuleKind, string, string][] = [
    ['domain-suffix', 'Domain and subdomains', 'example.org'],
    ['domain', 'Exact domain', 'api.github.com'],
    ['domain-keyword', 'Domain contains', 'google'],
    ['geosite', 'Geosite list', 'youtube'],
    ['geoip', 'Country (GeoIP)', 'DE'],
    ['ip-cidr', 'IP range', '203.0.113.0/24'],
    ['process-name', 'App (process name)', 'steam.exe'],
  ];
  const KIND_LABEL = Object.fromEntries(KINDS.map(([k, l]) => [k, l]));

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
    void app.saveSettings((x) => (x.core.custom_rules = [...x.core.custom_rules, rule]), 'Rule added');
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
      void app.saveSettings((x) => (x.core.custom_rules = next), 'Rule order saved');
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

  const remove = (i: number) => app.saveSettings((x) => (x.core.custom_rules = x.core.custom_rules.filter((_, k) => k !== i)));
  const polClass = (t: string) => (t === 'DIRECT' ? 'direct' : t === 'REJECT' ? 'reject' : 'proxy');
</script>

<div class="page-head">
  <h1>Rules</h1><span class="sub">The first matching rule decides where traffic goes</span>
</div>

{#if s}
  <div class="eyebrow">Routing presets</div>
  <div class="presets">
    {#each PRESETS as p (p.id)}
      {@const unavailable = p.needs === 'runetfreedom' && s.core.geodata.kind === 'default' && !s.core.presets.includes(p.id)}
      <div class="card preset">
        <div><div class="t">{p.title}</div><div class="d">{p.desc}</div>
          {#if unavailable}<button class="link" onclick={() => (app.view = 'settings')}>Switch routing data to Russia in Settings</button>{/if}</div>
        <button class="switch" role="switch" aria-label={p.title} aria-checked={s.core.presets.includes(p.id)} disabled={unavailable} onclick={() => togglePreset(p.id)}></button>
      </div>
    {/each}
  </div>

  <section class="card">
    <div class="card-h"><h2>Your rules</h2><span class="aside">checked before presets</span></div>
    <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <select class="input" bind:value={kind} aria-label="Match type">
        {#each KINDS as [k, l] (k)}<option value={k}>{l}</option>{/each}
      </select>
      <input class="input mono" bind:value={value} placeholder={KINDS.find((k) => k[0] === kind)?.[2]} aria-label="Value" />
      <select class="input" bind:value={target} aria-label="Send to">
        {#each targets as t (t)}<option value={t}>{t === 'Proxy' ? 'Proxy (main group)' : t}</option>{/each}
      </select>
      <button class="btn primary" type="submit" disabled={!value.trim()}><Icon name="plus" />Add rule</button>
    </form>
    {#if rules.length}
      <div class="twrap list">
        <table>
          <thead><tr><th></th><th>#</th><th>Match</th><th>Value</th><th>Send to</th><th></th></tr></thead>
          <tbody bind:this={tbody}>
            {#each rules as r, i (i)}
              <tr class:dragging={drag?.from === i} class:moving={drag !== null && drag.from !== i} style:transform={drag ? `translateY(${shift(i)}px)` : null}>
                <td class="grip-cell">
                  <button class="grip" aria-label="Move rule {i + 1}: drag, or use the arrow keys"
                    onpointerdown={(e) => grab(e, i)} onkeydown={(e) => gripKey(e, i)}><Icon name="grip" /></button>
                </td>
                <td class="num muted">{i + 1}</td>
                <td><span class="rt">{KIND_LABEL[r.kind] ?? r.kind}</span></td>
                <td class="mono small selectable">{r.value}</td>
                <td><span class="pol {polClass(r.target)}">{r.target}</span></td>
                <td class="r"><button class="x" onclick={() => remove(i)} aria-label="Delete rule {i + 1}"><Icon name="x" /></button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if rules.length > 1}<p class="hint">Drag <Icon name="grip" class="inline-grip" /> to change the order, or focus it and press ↑ or ↓. Rules higher up win.</p>{/if}
    {:else}
      <p class="hint">No rules of your own yet. Add one to send a site or app through the proxy, direct, or block it.</p>
    {/if}
    <div class="row-sw apply">
      <div><div class="t">Apply to imported Clash profiles</div><div class="d">Put your rules and presets in front of a Clash profile’s own rules{isClash ? ' (the active profile is one)' : ''}.</div></div>
      <button class="switch" role="switch" aria-label="Apply to imported Clash profiles" aria-checked={s.core.apply_rules_to_clash_profiles}
        onclick={() => app.saveSettings((x) => (x.core.apply_rules_to_clash_profiles = !x.core.apply_rules_to_clash_profiles))}></button>
    </div>
  </section>

  <p class="hint order">Order: your rules → presets → local network goes direct → everything else uses <b>{s.core.presets.includes('proxy-blocked-ru') ? 'DIRECT' : 'Proxy'}</b>. Global and Direct modes skip rules entirely.</p>
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
  .order b { color: var(--fg); font-weight: 600; }
</style>
