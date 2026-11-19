<script lang="ts">
  import { onDestroy } from 'svelte';
  import { countryCode, countryLabel, displayName } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import type { ProxyGroup } from '../lib/api';
  import Icon from './Icon.svelte';
  import Latency from './Latency.svelte';
  import Modal from './Modal.svelte';

  // The group, its members and how to choose come from the Dashboard, so the Quick switch
  // card and this window select in exactly one way.
  let { group, groupNames, servers, readOnly, pending, pick, groupTitle, groupSub, onclose }: {
    group: ProxyGroup;
    groupNames: string[];
    servers: string[];
    readOnly: boolean;
    pending: string | null;
    /** Resolves true when the choice took effect. */
    pick: (name: string) => Promise<boolean>;
    groupTitle: (name: string) => string;
    groupSub: (name: string) => string;
    onclose: () => void;
  } = $props();

  let q = $state('');
  type Sort = 'latency' | 'name' | 'profile';
  let sort = $state<Sort>('latency');
  let favOnly = $state(false);

  const delay = (n: string) => app.delayOf(n) ?? 1e9;
  // While a test runs the tiles keep their order, so a click never lands on one that moved;
  // the list is sorted again when the test ends. The user's own changes (sort, search,
  // favorites filter, a star) show at once, so they start a new frozen order.
  let frozen: string[] = [];
  let frozenFor = '';
  let shown = $derived.by(() => {
    const needle = q.trim().toLowerCase();
    const match = (n: string) => {
      if (!needle) return true;
      const info = app.nodes[n];
      return [n, info?.protocol, info?.transport, info?.server, countryCode(n)].join(' ').toLowerCase().includes(needle);
    };
    let list = servers.filter(match);
    if (sort === 'latency') list = [...list].sort((a, b) => delay(a) - delay(b));
    if (sort === 'name') list = [...list].sort((a, b) => displayName(a).localeCompare(displayName(b)));
    // Starred first in every order; the sorts are stable, so each keeps its own order inside.
    list = [...list.filter((n) => app.isFavorite(n)), ...(favOnly ? [] : list.filter((n) => !app.isFavorite(n)))];
    const key = [sort, needle, favOnly, servers.filter((n) => app.isFavorite(n)).join(',')].join('|');
    if (key !== frozenFor) { frozenFor = key; frozen = []; }
    if (app.testing.size > 0) {
      const keep = frozen.filter((n) => list.includes(n));
      list = [...keep, ...list.filter((n) => !keep.includes(n))];
    }
    frozen = list;
    return list;
  });
  let groupsShown = $derived(favOnly ? [] : groupNames.filter((n) => !q.trim() || [n, groupTitle(n)].join(' ').toLowerCase().includes(q.trim().toLowerCase())));

  const testAll = () => { if (app.testing.size === 0) void app.testDelays(); };
  // A choice that lands after this window closed must not close a window opened since.
  let alive = true;
  onDestroy(() => { alive = false; });
  async function choose(name: string) {
    if (readOnly || pending) return;
    if ((await pick(name)) && alive) onclose();
  }
  const manage = () => { app.proxiesGroup = group.name; onclose(); app.go('proxies'); };
</script>

<Modal title={t('Server selection')} wide {onclose}>
  <div class="toolbar">
    <label class="search"><Icon name="search" /><input class="input" placeholder={t('Search nodes, countries, protocols')} bind:value={q} aria-label={t('Search nodes')} /></label>
    <select class="input sort" bind:value={sort} aria-label={t('Sort')}>
      <option value="profile">{t('Profile order')}</option>
      <option value="latency">{t('Sort by latency')}</option>
      <option value="name">{t('Sort by name')}</option>
    </select>
    <button class="btn" aria-pressed={favOnly} class:on={favOnly} onclick={() => (favOnly = !favOnly)}><Icon name="star" />{t('Favorites')}</button>
    <button class="btn" class:busy={app.testing.size > 0} aria-disabled={app.testing.size > 0} onclick={testAll}><Icon name="bolt" />{t('Test latency')}</button>
  </div>
  {#if readOnly}<p class="muted note">{t('{group} chooses automatically. Pick a node in a select group to pin one.', { group: group.name })}</p>{/if}
  {#if groupsShown.length || shown.length}
    <div class="tiles" role="group" aria-label={t('Choose a server')}>
      {#each groupsShown as name (name)}
        <div class="wrap is-group">
          <button class="tile" aria-pressed={group.now === name} aria-disabled={readOnly || !!pending} onclick={() => choose(name)}>
            <span class="r1"><span class="cc"><Icon name="bolt" /></span><b title={name}>{groupTitle(name)}</b></span>
            <span class="r2"><span class="tr">{groupSub(name)}</span></span>
            <span class="r3">
              {#if pending === name}<span class="lat none">…</span>{:else}<Latency delay={app.delayOf(name)} dead={app.isDead(name)} testing={app.testing.has(app.resolve(name))} />{/if}
              {#if group.now === name}<Icon name="check" />{/if}
            </span>
          </button>
        </div>
      {/each}
      {#each shown as name (name)}
        {@const info = app.nodes[name]}
        {@const starred = app.isFavorite(name)}
        <div class="wrap">
          <button class="tile" aria-pressed={group.now === name} aria-disabled={readOnly || !!pending} onclick={() => choose(name)}>
            <span class="r1"><span class="cc">{countryLabel(name, app.flags)}</span><b title={name}>{displayName(name)}</b></span>
            <span class="r2">{#if info}<span class="tr">{info.protocol}{info.transport && info.transport !== 'tcp' ? ` · ${info.transport}` : ''}</span>{/if}</span>
            <span class="r3">
              {#if pending === name}<span class="lat none">…</span>{:else}<Latency delay={app.delayOf(name)} dead={app.isDead(name)} testing={app.testing.has(name)} />{/if}
              {#if group.now === name}<Icon name="check" />{/if}
            </span>
          </button>
          <!-- Beside the tile's button, not inside it: a button can't hold buttons. Only for servers
               the profile lists, as in Proxies, so a star set here can be taken off there. -->
          {#if info}<button class="star" class:on={starred} aria-pressed={starred} title={starred ? t('Remove from favorites') : t('Add to favorites')} aria-label={starred ? t('Remove from favorites') : t('Add to favorites')} onclick={() => app.toggleFavorite(name)}><Icon name="star" /></button>{/if}
        </div>
      {/each}
    </div>
  {:else}
    <p class="hint">{favOnly && !q.trim() ? t('No favorite servers yet. Press the star on a server to add it.') : t('No nodes match “{q}”.', { q })}</p>
  {/if}
  {#snippet footer()}
    <button class="btn ghost manage" onclick={manage}>{t('Manage in Proxies')}</button>
    <button class="btn ghost" onclick={onclose}>{t('Close')}</button>
  {/snippet}
</Modal>

<style>
  /* The body is a column that may shrink: only the tiles give way, never the toolbar. */
  .toolbar { flex: none; display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .toolbar .search { flex: 1 1 200px; min-width: 0; }
  .toolbar .search .input { width: 100%; }
  .sort { width: auto; }
  .btn.on { background: var(--accent-soft); border-color: var(--faint); }
  .btn[aria-disabled='true'] { opacity: .55; cursor: default; }
  .note { flex: none; margin: 0; font-size: 12px; }
  .hint { margin: 0; }
  /* The tiles scroll under the toolbar; the small padding keeps the accent border from being clipped. */
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(200px, 100%), 1fr)); gap: 8px; max-height: 52vh; overflow-y: auto; padding: 1px; }
  @media (max-width: 420px) { .tiles { grid-template-columns: minmax(0, 1fr); } }
  .wrap { position: relative; display: flex; min-width: 0; }
  .tile { flex: 1; display: flex; flex-direction: column; gap: 6px; min-height: 44px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 10px; background: var(--sunk); color: var(--muted); font: inherit; text-align: start; min-width: 0; }
  .tile[aria-pressed='true'] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); background: var(--accent-soft); color: var(--fg); }
  .tile[aria-disabled='true'] { cursor: default; }
  @media (hover: hover) { .tile:not([aria-pressed='true']):not([aria-disabled='true']):hover { border-color: var(--line-2); color: var(--fg); } }
  .r1 { display: flex; align-items: center; gap: 8px; min-width: 0; padding-inline-end: 28px; }
  .r1 b { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; font-size: 13px; color: var(--fg); }
  .is-group .r1 { padding-inline-end: 0; }
  .is-group .cc { background: var(--accent); color: var(--accent-fg); }
  .r2 { min-height: 16px; min-width: 0; }
  .tr { display: block; font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r3 { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 22px; }
  .r3 > :global(.i) { width: 16px; height: 16px; color: var(--accent); }
  .star { position: absolute; inset-inline-end: 8px; top: 8px; width: 26px; height: 26px; border: 0; border-radius: 6px; background: none; color: var(--faint); display: grid; place-items: center; padding: 0; }
  .star :global(.i) { width: 15px; height: 15px; }
  .star.on { color: var(--fg); }
  .star.on :global(.i) { fill: currentColor; }
  @media (hover: hover) {
    .star:hover { background: var(--panel); color: var(--fg); }
    .star:not(.on) { opacity: 0; pointer-events: none; }
    .wrap:hover .star, .wrap:focus-within .star { opacity: 1; pointer-events: auto; }
  }
  /* Fingers need bigger targets than a pointer. */
  @media (hover: none) {
    /* Beside the tile, not over its corner, so a tap near the star never picks the server. */
    .star { position: static; flex: none; width: 44px; height: 44px; border-radius: 10px; }
    .star :global(.i) { width: 18px; height: 18px; }
    .r1 { padding-inline-end: 0; }
  }
  .manage { margin-inline-end: auto; }
  :global(.app.mobile) .tile, :global(.app.tv) .tile { min-height: 44px; }
</style>
