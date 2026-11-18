<script lang="ts">
  import { onDestroy } from 'svelte';
  import { countryCode, countryLabel, displayName } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';
  import Latency from '../components/Latency.svelte';
  import NodeModal from '../components/NodeModal.svelte';

  const KIND: Record<string, string> = { Selector: 'select', URLTest: 'url-test', Fallback: 'fallback', LoadBalance: 'load-balance', Relay: 'relay' };
  let NOTE = $derived<Record<string, string>>({
    Selector: t('You choose which member carries traffic.'),
    URLTest: t('Re-tests every few minutes and moves to the fastest member.'),
    Fallback: t('Uses the first member that passes health checks.'),
    LoadBalance: t('Spreads connections across members.'),
  });

  // How servers show, kept with the layout for this device.
  let view = $derived(app.layout.proxies!);
  let viewOpen = $state(false);
  const setView = (k: 'compact' | 'protocol' | 'transport' | 'cores' | 'address') => app.saveLayout({ ...app.layout, proxies: { ...view, [k]: !view[k] } });

  // A window that sent the user here names the group it was showing; it counts once.
  let groupName = $state<string | null>(app.proxiesGroup);
  app.proxiesGroup = null;
  // One column (a phone held upright): only the chosen group shows, so its servers are not
  // pushed below a list of every group. Tapping it opens the list.
  let groupsOpen = $state(false);
  let q = $state('');
  type Sort = 'latency' | 'name' | 'profile';
  // The chosen order is kept between visits and restarts.
  let sort = $state<Sort>((() => { try { const v = localStorage.getItem('kova.proxySort'); return v === 'latency' || v === 'name' ? v : 'profile'; } catch { return 'profile'; } })());
  $effect(() => { try { localStorage.setItem('kova.proxySort', sort); } catch { /* back to the default next time */ } });

  // Only the starred servers, kept between visits like the order.
  let favOnly = $state((() => { try { return localStorage.getItem('kova.proxyFav') === '1'; } catch { return false; } })());
  $effect(() => { try { localStorage.setItem('kova.proxyFav', favOnly ? '1' : '0'); } catch { /* shows all next time */ } });
  // The server that opens in the details window.
  let detailsOf = $state<string | null>(null);
  /** `host:port`, with an IPv6 host in brackets. */
  const address = (host: string, port: number) => `${host.includes(':') && !host.startsWith('[') ? `[${host}]` : host}:${port}`;

  let groups = $derived(app.proxies.groups.filter((g) => g.name !== 'GLOBAL' || app.settings?.core.mode === 'global'));
  let group = $derived(groups.find((g) => g.name === groupName) ?? groups[0]);
  let isGroup = (n: string) => app.proxies.groups.some((g) => g.name === n);
  // Servers a proxy provider downloads are only known to the running core; until then
  // the profile's own list is all there is.
  const NOT_A_SERVER = new Set(['Direct', 'Reject', 'RejectDrop', 'Pass', 'Compatible', 'Selector', 'URLTest', 'Fallback', 'LoadBalance', 'Relay']);
  let nodeCount = $derived(new Set([...Object.keys(app.nodes), ...Object.values(app.proxies.proxies).filter((p) => !NOT_A_SERVER.has(p.type) && !isGroup(p.name)).map((p) => p.name)]).size);

  let members = $derived.by(() => {
    if (!group) return [];
    const needle = q.trim().toLowerCase();
    let list = group.all.filter((n) => {
      if (!needle) return true;
      const info = app.nodes[n];
      return [n, info?.protocol, info?.transport, info?.server, countryCode(n)].join(' ').toLowerCase().includes(needle);
    });
    if (sort === 'latency') list = [...list].sort((a, b) => Number(isGroup(b)) - Number(isGroup(a)) || (app.delayOf(a) ?? 1e9) - (app.delayOf(b) ?? 1e9));
    if (sort === 'name') list = [...list].sort((a, b) => displayName(a).localeCompare(displayName(b)));
    // Starred servers first in every order (the sorts above are stable, so each keeps its own order inside).
    const starred = (n: string) => app.isFavorite(n);
    list = [...list.filter(starred), ...(favOnly ? [] : list.filter((n) => !starred(n)))];
    return list;
  });

  function pick(name: string) {
    if (!group) return;
    if (group.type !== 'Selector') {
      app.notify(t('{group} chooses automatically. Pick a node in a select group to pin one.', { group: group.name }));
      return;
    }
    void app.select(group.name, name);
  }

  function testGroup() {
    if (!group) return;
    const names = group.all.filter((n) => !isGroup(n) && !['DIRECT', 'REJECT'].includes(n));
    void app.testDelays(names.length ? names : undefined);
  }

  // Keep group selections fresh while the core moves url-test groups around.
  const poll = setInterval(() => { if (app.status.running && !document.hidden) void app.refreshProxies(); }, 5000);
  onDestroy(() => clearInterval(poll));
  void app.refreshProxies();
</script>

<div class="page-head">
  <h1>{t('Proxies')}</h1>
  <span class="sub">{tn(groups.length, '{n} group', '{n} groups')} · {tn(nodeCount, '{n} node', '{n} nodes')}</span>
  <div class="tools">
    <label class="search"><Icon name="search" /><input class="input" placeholder={t('Search nodes, countries, protocols')} bind:value={q} aria-label={t('Search nodes')} /></label>
    <select class="input sort" bind:value={sort} aria-label={t('Sort')}>
      <option value="profile">{t('Profile order')}</option>
      <option value="latency">{t('Sort by latency')}</option>
      <option value="name">{t('Sort by name')}</option>
    </select>
    <button class="btn" aria-pressed={favOnly} class:on={favOnly} onclick={() => (favOnly = !favOnly)}><Icon name="star" />{t('Favorites')}</button>
    <div class="view-menu">
      <button class="btn" aria-expanded={viewOpen} onclick={() => (viewOpen = !viewOpen)}><Icon name="settings" />{t('View')}</button>
      {#if viewOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="view-scrim" onclick={() => (viewOpen = false)}></div>
        <div class="view-pop" role="group" aria-label={t('View')}>
          <label class="vrow"><input type="checkbox" checked={view.compact} onchange={() => setView('compact')} />{t('Compact rows')}</label>
          <label class="vrow" class:dim={view.compact}><input type="checkbox" checked={view.protocol} disabled={view.compact} onchange={() => setView('protocol')} />{t('Protocol')}</label>
          <label class="vrow" class:dim={view.compact}><input type="checkbox" checked={view.transport} disabled={view.compact} onchange={() => setView('transport')} />{t('Transport and security')}</label>
          <label class="vrow" class:dim={view.compact}><input type="checkbox" checked={view.cores} disabled={view.compact} onchange={() => setView('cores')} />{t('Which cores run it')}</label>
          <label class="vrow" class:dim={view.compact}><input type="checkbox" checked={view.address} disabled={view.compact} onchange={() => setView('address')} />{t('Server address')}</label>
        </div>
      {/if}
    </div>
    <button class="btn primary" onclick={testGroup} disabled={!group || app.testing.size > 0}><Icon name="bolt" />{t('Test latency')}</button>
  </div>
</div>

{#if !groups.length}
  <div class="empty">
    <b>{t('No nodes yet')}</b>
    <span>{t('Add a subscription or share links in Profiles.')}</span>
    <button class="btn primary" onclick={() => { app.go('profiles'); app.importOpen = true; }}><Icon name="plus" />{t('Add profile')}</button>
  </div>
{:else}
  <div class="px">
    <div class="groups" class:open={groupsOpen}>
      {#each groups as g (g.name)}
        {@const end = app.resolve(g.name)}
        <button class="grp" onclick={() => { if (g.name === group?.name) groupsOpen = !groupsOpen; else { groupName = g.name; groupsOpen = false; } }} aria-current={g.name === group?.name} aria-expanded={g.name === group?.name ? groupsOpen : undefined}>
          <span class="top"><b>{g.name}</b><span class="chip mono">{KIND[g.type] ?? g.type.toLowerCase()}</span>{#if g.name === group?.name}<span class="more" aria-hidden="true">{groupsOpen ? '▴' : '▾'} {groups.length}</span>{/if}</span>
          <span class="now">{g.now && g.now !== end ? `${displayName(g.now)} › ` : ''}{displayName(end)}{app.delays[end] != null ? ` · ${t('{n} ms', { n: app.delays[end]! })}` : ''}</span>
        </button>
      {/each}
    </div>
    <div>
      {#if group}
        <p class="grp-note">{NOTE[group.type] ?? ''}{#if group.type !== 'Selector'} <span class="chip">{t('Automatic')}</span>{/if}</p>
        {#if members.length}
          <div class="nodes" class:compact={view.compact}>
            {#each members as name (name)}
              {@const info = app.nodes[name]}
              {@const sub = isGroup(name)}
              {@const real = !sub && !!info}
              <div class="node-wrap" class:is-group={sub} class:has-acts={real}>
                <button class="node" aria-pressed={group.now === name} onclick={() => pick(name)}>
                  <span class="r1">
                    <span class="cc">{#if sub}<Icon name="bolt" />{:else}{countryLabel(name, app.flags)}{/if}</span>
                    <b title={name}>{displayName(name)}</b>
                    <Latency delay={app.delayOf(name)} dead={app.isDead(name)} testing={app.testing.has(app.resolve(name))} />
                  </span>
                  {#if sub}
                    <span class="r2"><span class="chip mono">{t('group')}</span><span class="tr">{t('now {name}', { name: displayName(app.resolve(name)) })}</span></span>
                  {:else if info}
                    {#if !view.compact && (view.protocol || view.transport)}
                      <span class="r2">{#if view.protocol}<span class="chip">{info.protocol}</span>{/if}{#if view.transport}<span class="tr">{info.transport}</span>{/if}</span>
                    {/if}
                    {#if !view.compact && view.address}
                      <span class="addr mono" title={address(info.server, info.port)}>{address(info.server, info.port)}</span>
                    {/if}
                    {#if !view.compact && view.cores}
                      <span class="r3"><span class="dot" class:y={info.mihomo}></span>Mihomo <span class="dot" class:y={info.xray}></span>Xray
                        {#if app.status.bridged.includes(name)}<span class="via">· {t('via Xray bridge')}</span>{/if}</span>
                    {/if}
                  {:else}
                    <span class="r2"><span class="tr">{app.proxies.proxies[name]?.type ?? ''}</span></span>
                  {/if}
                </button>
                {#if real}
                  <!-- Beside the card's button, not inside it: a button can't hold buttons. -->
                  <div class="acts">
                    <button class="act star" class:on={app.isFavorite(name)} aria-pressed={app.isFavorite(name)} title={app.isFavorite(name) ? t('Remove from favorites') : t('Add to favorites')} aria-label={app.isFavorite(name) ? t('Remove from favorites') : t('Add to favorites')} onclick={() => app.toggleFavorite(name)}><Icon name="star" /></button>
                    <button class="act eye" title={t('Show configuration')} aria-label={t('Show configuration')} onclick={() => (detailsOf = name)}><Icon name="eye" /></button>
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {:else}
          <p class="hint">{favOnly && !q.trim() ? t('No favorite servers yet. Press the star on a server to add it.') : t('No nodes match “{q}”.', { q })}</p>
        {/if}
      {/if}
    </div>
  </div>
{/if}

{#if detailsOf}<NodeModal name={detailsOf} onclose={() => (detailsOf = null)} />{/if}

<style>
  .sort { width: auto; }
  .px { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: 16px; align-items: start; }
  @container (max-width: 700px) {
    .px { grid-template-columns: minmax(0, 1fr); }
    .px .groups { position: static; }
    .px .groups:not(.open) .grp:not([aria-current='true']) { display: none; }
    .px .groups.open { max-height: 60vh; overflow: auto; }
    .px .more { display: inline; margin-inline-start: 6px; font: 500 11px var(--f-mono); color: var(--muted); }
  }
  .more { display: none; }
  .groups { display: flex; flex-direction: column; gap: 6px; position: sticky; top: 0; }
  .grp { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); text-align: start; min-width: 0; }
  .grp:hover { border-color: var(--line-2); }
  .grp[aria-current='true'] { border-color: var(--faint); background: var(--accent-soft); }
  .top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .top b { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .top .chip { margin-inline-start: auto; height: 20px; font-size: 11px; }
  .now { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grp-note { font-size: 12px; color: var(--muted); margin: 0 0 12px; display: flex; gap: 8px; align-items: center; min-height: 22px; }
  .nodes { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 10px; }
  /* The wrapper is the card; its button selects the server, the small buttons sit over the first row. */
  .node-wrap { position: relative; display: flex; min-width: 0; border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); }
  .node { flex: 1; display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 0; border-radius: inherit; background: none; color: inherit; font: inherit; text-align: start; min-width: 0; }
  .node-wrap:hover { border-color: var(--line-2); }
  .node-wrap:has(.node[aria-pressed='true']) { border-color: var(--faint); background: var(--accent-soft); }
  /* Room in the first row for the star and the eye. */
  .has-acts .r1 { padding-inline-end: 48px; }
  .acts { position: absolute; inset-inline-end: 8px; top: 9px; display: flex; gap: 0; }
  .act { width: 24px; height: 24px; border: 0; border-radius: 6px; background: none; color: var(--faint); display: grid; place-items: center; padding: 0; flex: none; }
  .act :global(.i) { width: 15px; height: 15px; }
  .act.on { color: var(--fg); }
  .act.on :global(.i) { fill: currentColor; }
  @media (hover: hover) {
    .act:hover { background: var(--sunk); color: var(--fg); }
    .act.star:not(.on), .act.eye { opacity: 0; pointer-events: none; }
    .node-wrap:hover .act, .node-wrap:focus-within .act { opacity: 1; pointer-events: auto; }
  }
  /* Fingers need bigger targets than a pointer. */
  @media (hover: none) {
    .act { width: 30px; height: 30px; }
    .has-acts .r1 { padding-inline-end: 64px; }
    .acts { inset-inline-end: 6px; top: 6px; }
  }
  .addr { font-size: 11px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .btn.on { background: var(--accent-soft); border-color: var(--faint); }
  .r1 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .r1 b { font-weight: 500; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r2 { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; min-width: 0; }
  .tr { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r3 { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--faint); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--line-2); }
  .dot.y { background: var(--fg); }
  .via { color: var(--muted); }
  .is-group .cc { background: var(--accent); color: var(--accent-fg); }
  .nodes.compact { grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 6px; }
  .nodes.compact .node { padding: 8px 10px; gap: 4px; }
  .nodes.compact .acts { top: 5px; inset-inline-end: 6px; }
  .nodes.compact .r2 { display: none; }
  .view-menu { position: relative; }
  .view-scrim { position: fixed; inset: 0; z-index: 19; }
  .view-pop { position: absolute; inset-inline-end: 0; top: calc(100% + 6px); z-index: 20; min-width: 240px; display: flex; flex-direction: column; padding: 6px; border: 1px solid var(--line-2); border-radius: var(--r); background: var(--panel); box-shadow: var(--shadow); }
  .vrow { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 6px; font-size: 13px; cursor: pointer; }
  @media (hover: hover) { .vrow:hover { background: var(--sunk); } }
  .vrow.dim { opacity: .5; }
</style>
