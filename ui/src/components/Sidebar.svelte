<script lang="ts">
  import { daysUntil } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app, type View } from '../lib/state.svelte';
  import Icon from './Icon.svelte';

  let NAV = $derived<[View, string][]>([
    ['dashboard', t('Dashboard')], ['proxies', t('Proxies')], ['profiles', t('Profiles')], ['connections', t('Connections')],
    ['rules', t('Rules')], ['check', t('Site check')], ['logs', t('Logs')], ['settings', t('Settings')],
  ]);

  let nodeCount = $derived(app.activeProfile?.node_count);
  let usage = $derived(app.activeProfile?.usage);
  let used = $derived(usage ? usage.upload + usage.download : 0);
  let pct = $derived(usage && usage.total ? Math.min(100, (used / usage.total) * 100) : 0);
  const gb = (b: number) => (b / 1024 ** 3).toFixed(b < 10 * 1024 ** 3 ? 1 : 0);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
{#if app.mobile && !app.tv && app.navOpen}<div class="scrim" onclick={() => app.closeNav()}></div>{/if}
<aside class="side" class:open={app.navOpen}>
  <nav class="nav">
    {#each NAV as [id, label] (id)}
      <button onclick={() => app.go(id)} aria-current={app.view === id ? 'page' : undefined}>
        <Icon name={id === 'check' ? 'pulse' : id} />{label}
        {#if id === 'proxies' && nodeCount != null}<span class="badge">{nodeCount}</span>{/if}
        {#if id === 'profiles'}<span class="badge">{app.profiles.length}</span>{/if}
        {#if id === 'rules' && app.suggestionCount}<span class="badge new" title={t('Suggestions waiting')}>{app.suggestionCount}</span>{/if}
      </button>
    {/each}
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
        <div class="s"><span>{app.activeProfile.source.kind === 'remote' ? t('Subscription') : t('Local')}</span><span>{tn(app.activeProfile.node_count, '{n} node', '{n} nodes')}</span></div>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .side { border-right: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; padding: 12px 10px; min-height: 0; overflow: auto; }
  .nav { display: flex; flex-direction: column; gap: 2px; }
  .nav button { display: flex; align-items: center; gap: 11px; padding: 8px 10px; border: 0; border-radius: 8px; background: none; color: var(--muted); font-weight: 500; text-align: start; white-space: nowrap; }
  @media (hover: hover) { .nav button:hover { background: var(--sunk); color: var(--fg); } }
  .nav button[aria-current='page'] { background: var(--accent-soft); color: var(--fg); }
  .badge { margin-inline-start: auto; font: 500 11px var(--f-mono); color: var(--faint); }
  .badge.new { color: var(--accent-fg); background: var(--accent); border-radius: 9px; padding: 0 6px; line-height: 17px; }
  .update { margin-top: auto; display: flex; align-items: center; gap: 8px; padding: 8px 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--accent-soft); color: var(--fg); font-size: 12.5px; font-weight: 500; text-align: start; }
  .update :global(.i) { width: 15px; height: 15px; }
  .update + .sub-card { margin-top: 0; }
  .sub-card { margin-top: auto; border: 1px solid var(--line); border-radius: var(--r); padding: 11px 12px; background: var(--panel); display: flex; flex-direction: column; gap: 7px; }
  .t { font-weight: 600; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s { font-size: 12px; color: var(--muted); display: flex; justify-content: space-between; gap: 8px; }

  /* Phone: the same list as a panel that slides in from the left, over the page. A TV
     has the room to keep it in place, as on a computer. */
  .scrim { position: fixed; inset: 0; background: rgb(0 0 0 / .5); z-index: 24; }
  :global(.app.mobile:not(.tv)) .side { position: fixed; top: 0; bottom: 0; left: 0; width: min(300px, 82vw); z-index: 25; background: var(--win); padding: 16px 12px; gap: 14px;
    transform: translateX(-102%); transition: transform .2s ease; box-shadow: var(--shadow); }
  :global(.app.mobile:not(.tv)) .side.open { transform: none; }
  :global([dir='rtl'] .app.mobile:not(.tv)) .side { left: auto; right: 0; transform: translateX(102%); }
  :global([dir='rtl'] .app.mobile:not(.tv)) .side.open { transform: none; }
  :global(.app.mobile) .nav button { padding: 13px 12px; font-size: 15px; gap: 14px; }
  :global(.app.mobile) .nav button :global(.i) { width: 20px; height: 20px; }
</style>
