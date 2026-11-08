<script lang="ts">
  import { COMBINED } from '../lib/api';
  import { daysUntil } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app, type View } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import { move, type TabId } from '../lib/layout';
  import { sortable } from '../lib/sortable';
  import { pictureStyle } from '../lib/pictures';

  let LABEL = $derived<Record<View, string>>({
    dashboard: t('Dashboard'), proxies: t('Proxies'), profiles: t('Profiles'), connections: t('Connections'),
    rules: t('Rules'), check: t('Site check'), logs: t('Logs'), settings: t('Settings'),
  });
  // Dashboard first and Settings last always; the tabs between follow the user's layout.
  let tabs = $derived(app.layout.tabs);
  let shownTabs = $derived(app.editingLayout ? tabs : tabs.filter((x) => !x.hidden));
  const moveTab = (from: number, to: number) => app.saveLayout({ ...app.layout, tabs: move(app.layout.tabs, from, to) });
  const toggleTab = (id: TabId) => app.saveLayout({ ...app.layout, tabs: app.layout.tabs.map((x) => (x.id === id ? { id, ...(x.hidden ? {} : { hidden: true }) } : x)) });

  let sidePic = $derived(app.settings?.look?.pictures?.sidebar);
  let sideUrl = $derived(app.pictureUrl(sidePic?.file));
  let nodeCount = $derived(app.activeProfile?.node_count);
  let usage = $derived(app.activeProfile?.usage);
  let used = $derived(usage ? usage.upload + usage.download : 0);
  let pct = $derived(usage && usage.total ? Math.min(100, (used / usage.total) * 100) : 0);
  const gb = (b: number) => (b / 1024 ** 3).toFixed(b < 10 * 1024 ** 3 ? 1 : 0);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
{#if app.mobile && !app.tv && app.navOpen}<div class="scrim" onclick={() => app.closeNav()}></div>{/if}
<aside class="side" class:open={app.navOpen} style={sidePic && sideUrl ? pictureStyle(sidePic, sideUrl) : undefined}>
  {#snippet tab(id: View)}
    <button class="go" onclick={() => app.go(id)} aria-current={app.view === id ? 'page' : undefined}>
      <Icon name={id === 'check' ? 'pulse' : id} />{LABEL[id]}
      {#if !app.editingLayout}
        {#if id === 'proxies' && nodeCount != null}<span class="badge">{nodeCount}</span>{/if}
        {#if id === 'profiles'}<span class="badge">{app.profiles.length}</span>{/if}
        {#if id === 'rules' && app.rulesDraft}<span class="badge new" title={t('Not applied yet')}>•</span>
        {:else if id === 'rules' && app.suggestionCount}<span class="badge new" title={t('Suggestions waiting')}>{app.suggestionCount}</span>{/if}
      {/if}
    </button>
  {/snippet}
  <nav class="nav" class:editing={app.editingLayout}>
    {@render tab('dashboard')}
    <div class="tabs" use:sortable={{ enabled: app.editingLayout, onmove: moveTab }}>
      {#each shownTabs as x (x.id)}
        <div class="tab" data-sort-id={x.id} class:off={x.hidden}>
          {@render tab(x.id)}
          {#if app.editingLayout}
            <button class="eye" onclick={() => toggleTab(x.id)} aria-pressed={!x.hidden} aria-label={x.hidden ? t('Show {name}', { name: LABEL[x.id] }) : t('Hide {name}', { name: LABEL[x.id] })} title={x.hidden ? t('Show') : t('Hide')}><Icon name={x.hidden ? 'eye-off' : 'eye'} /></button>
            <button class="handle" data-handle aria-label={t('Move {name}', { name: LABEL[x.id] })} title={t('Drag to move')}><Icon name="grip" /></button>
          {/if}
        </div>
      {/each}
    </div>
    {@render tab('settings')}
  </nav>
  {#if app.update && !app.mobile}
    <button class="update" onclick={() => app.go('settings')}><Icon name="down" />{t('Kova {version} is available', { version: app.update.version })}</button>
  {/if}
  {#if app.activeProfile}
    <div class="sub-card">
      <div class="eyebrow">{t('Active profile')}</div>
      <div class="t">{app.activeProfile.name}</div>
      {#if usage && usage.total}
        <div class="bar" class:warn={pct > 85}><i style="width:{pct}%"></i></div>
        <div class="s"><span class="num">{gb(used)} / {gb(usage.total)} GB</span>{#if usage.expire}<span>{tn(daysUntil(usage.expire), '{n} day left', '{n} days left')}</span>{/if}</div>
      {:else}
        <div class="s"><span>{app.activeProfile.id === COMBINED ? tn(app.combinedMembers.length, '{n} profile', '{n} profiles') : app.activeProfile.source.kind === 'remote' ? t('Subscription') : t('Local')}</span><span>{tn(app.activeProfile.node_count, '{n} node', '{n} nodes')}</span></div>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .side { border-right: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; padding: 12px 10px; min-height: 0; overflow: auto; }
  .nav { display: flex; flex-direction: column; gap: 2px; }
  .tabs { display: flex; flex-direction: column; gap: 2px; }
  .tab { display: flex; align-items: center; gap: 2px; border-radius: 8px; }
  .tab .go { flex: 1; min-width: 0; }
  .tab.off .go { opacity: .45; }
  .nav .eye, .nav .handle { flex: none; padding: 6px; color: var(--faint); }
  .nav .handle { cursor: grab; touch-action: none; }
  .tab:global(.dragging) { opacity: .6; }
  .tab:global(.drop-target) { box-shadow: inset 0 0 0 1.5px var(--accent); }
  .nav.editing .tab { background: var(--sunk); }
  .nav button { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border: 0; border-radius: 8px; background: none; color: var(--muted); font-weight: 500; text-align: start; white-space: nowrap; }
  @media (hover: hover) { .nav button:hover { background: var(--sunk); color: var(--fg); } }
  .nav button[aria-current='page'] { background: var(--accent-soft); color: var(--fg); }
  .badge { margin-inline-start: auto; font: 500 11px var(--f-mono); color: var(--faint); }
  .badge.new { color: var(--accent-fg); background: var(--accent); border-radius: 9px; padding: 0 6px; line-height: 17px; }
  .update { margin-top: auto; display: flex; align-items: center; gap: 8px; padding: 8px 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--accent-soft); color: var(--fg); font-size: 12px; font-weight: 500; text-align: start; }
  .update :global(.i) { width: 15px; height: 15px; }
  .update + .sub-card { margin-top: 0; }
  .sub-card { margin-top: auto; border: 1px solid var(--line); border-radius: var(--r); padding: 12px 12px; background: var(--panel); display: flex; flex-direction: column; gap: 8px; }
  .t { font-weight: 500; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s { font-size: 12px; color: var(--muted); display: flex; justify-content: space-between; gap: 8px; }

  /* Phone: the same list as a panel that slides in from the left, over the page. A TV
     has the room to keep it in place, as on a computer. */
  .scrim { position: fixed; inset: 0; background: rgb(0 0 0 / .5); z-index: 24; }
  :global(.app.mobile:not(.tv)) .side { position: fixed; top: 0; bottom: 0; left: 0; width: min(300px, 82vw); z-index: 25; background: var(--win); padding: 16px 12px; gap: 14px;
    transform: translateX(-102%); transition: transform .2s ease; box-shadow: var(--shadow); }
  :global(.app.mobile:not(.tv)) .side.open { transform: none; }
  :global([dir='rtl'] .app.mobile:not(.tv)) .side { left: auto; right: 0; transform: translateX(102%); }
  :global([dir='rtl'] .app.mobile:not(.tv)) .side.open { transform: none; }
  :global(.app.mobile) .nav button { padding: 12px 12px; font-size: 15px; gap: 14px; }
  :global(.app.mobile) .nav button :global(.i) { width: 20px; height: 20px; }
</style>
