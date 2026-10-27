<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api, type CorePreference, type Profile } from '../lib/api';
  import { daysUntil, fmtAgo, fmtDate } from '../lib/format';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import EditorModal from '../components/EditorModal.svelte';
  import ShareModal from '../components/ShareModal.svelte';
  import Icon from '../components/Icon.svelte';

  let selectedId = $state<string | null>(null);
  let editing = $state<Profile | null>(null);
  let sharing = $state<Profile | null>(null);
  let confirmDelete = $state(false);
  let updating = $state<Set<string>>(new Set());

  let sel = $derived(app.profiles.find((p) => p.id === (selectedId ?? app.settings?.active_profile)) ?? app.profiles[0]);
  $effect(() => { void sel?.id; confirmDelete = false; });

  let FORMAT = $derived<Record<Profile['format'], string>>({ clash: 'Clash / Mihomo YAML', 'sing-box': 'sing-box JSON', links: t('Share links') });
  const gb = (b: number) => (b / 1024 ** 3).toFixed(b < 10 * 1024 ** 3 ? 1 : 0);

  async function update(id: string) {
    updating = new Set([...updating, id]);
    await app.run(() => api.updateProfile(id), (p) => tn(p.node_count, '{name} updated · {n} node', '{name} updated · {n} nodes', { name: p.name }));
    const u = new Set(updating); u.delete(id); updating = u;
  }

  async function updateAll() {
    const ids = app.profiles.filter((p) => p.source.kind === 'remote').map((p) => p.id);
    updating = new Set(ids);
    const failed = await app.run(() => api.updateAllProfiles());
    updating = new Set();
    if (failed) app.notify(failed.length ? t('{n} failed: {name} ({error})', { n: failed.length, name: failed[0][0], error: failed[0][1] }) : tn(ids.length, 'Updated {n} subscription', 'Updated {n} subscriptions'));
  }

  const edit = (patch: Parameters<typeof api.editProfile>[1]) => sel && app.run(() => api.editProfile(sel!.id, patch));

  async function remove() {
    if (!sel) return;
    const name = sel.name;
    await app.run(() => api.removeProfile(sel!.id), t('Deleted {name}', { name }));
    selectedId = null;
  }
</script>

<div class="page-head">
  <h1>{t('Profiles')}</h1><span class="sub">{t('Subscriptions and local configs')}</span>
  <div class="tools">
    {#if app.profiles.some((p) => p.source.kind === 'remote')}
      <button class="btn" class:busy={updating.size > 0} onclick={updateAll} disabled={updating.size > 0}><Icon name="refresh" />{t('Update all')}</button>
    {/if}
    <button class="btn primary" onclick={() => (app.importOpen = true)}><Icon name="plus" />{t('Add profile')}</button>
  </div>
</div>

{#if !app.profiles.length}
  <div class="empty">
    <b>{t('No profiles yet')}</b>
    <span>{t('Paste a subscription URL from your provider, or share links such as vless:// and hy2://.')}</span>
    <button class="btn primary" onclick={() => (app.importOpen = true)}><Icon name="plus" />{t('Add profile')}</button>
  </div>
{:else}
  <div class="pf">
    <div class="plist">
      {#each app.profiles as p (p.id)}
        {@const used = p.usage ? p.usage.upload + p.usage.download : 0}
        {@const pct = p.usage?.total ? Math.min(100, (used / p.usage.total) * 100) : 0}
        {@const left = p.usage?.expire ? daysUntil(p.usage.expire) : null}
        {@const active = p.id === app.settings?.active_profile}
        <button class="prof" aria-current={p.id === sel?.id} onclick={() => (selectedId = p.id)}>
          <span class="radio" class:on={active}></span>
          <span class="t">
            <b>{p.name}</b>
            {#if active}<span class="chip acc">{t('In use')}</span>{/if}
            {#if left != null && left <= 14}<span class="chip warn">{left <= 0 ? t('Expired') : tn(left, 'Expires in {n} day', 'Expires in {n} days')}</span>{/if}
            {#if updating.has(p.id)}<span class="chip">{t('Updating…')}</span>{/if}
          </span>
          <span class="m"><span>{p.source.kind === 'remote' ? t('Subscription') : t('Local')}</span><span>{FORMAT[p.format]}</span><span>{tn(p.node_count, '{n} node', '{n} nodes')}</span><span>{fmtAgo(p.updated_at)}</span></span>
          {#if p.usage?.total}
            <span class="u">
              <span class="bar" class:warn={pct > 85}><i style="width:{pct}%"></i></span>
              <span class="s"><span class="num">{t('{used} GB of {total} GB', { used: gb(used), total: gb(p.usage.total) })}</span>{#if p.usage.expire}<span>{t('until {date}', { date: fmtDate(p.usage.expire) })}</span>{/if}</span>
            </span>
          {/if}
        </button>
      {/each}
    </div>

    {#if sel}
      {#key sel.id}
        <section class="card detail">
          <div class="card-h"><h2>{sel.name}</h2><span class="aside">{sel.source.kind === 'remote' ? t('Subscription') : t('Local profile')}</span></div>
          <div class="form">
            <div class="fld full"><label for="pf-name">{t('Name')}</label>
              <input id="pf-name" class="input" value={sel.name} onchange={(e) => edit({ name: e.currentTarget.value })} /></div>
            {#if sel.source.kind === 'remote'}
              <div class="fld full"><label for="pf-url">{t('Subscription URL')}</label>
                <input id="pf-url" class="input mono small" value={sel.source.url} onchange={(e) => edit({ url: e.currentTarget.value })} /></div>
            {/if}
            <div class="fld"><label for="pf-core">{t('Run on core')}</label>
              <select id="pf-core" class="input" value={sel.core} onchange={(e) => edit({ core: e.currentTarget.value as CorePreference })}>
                <option value="auto">{t('Automatic')}</option><option value="mihomo">{t('Mihomo only')}</option>
                <!-- On a phone the VPN needs Mihomo in front; Xray still runs what only it supports. -->
                {#if !app.mobile || sel.core === 'xray'}<option value="xray">{t('Xray only')}</option>{/if}
              </select></div>
            {#if sel.source.kind === 'remote'}
              <div class="fld"><label for="pf-every">{t('Auto-update')}</label>
                <select id="pf-every" class="input" value={String(sel.source.update_hours)} onchange={(e) => edit({ update_hours: Number(e.currentTarget.value) })}>
                  <option value="0">{t('Off')}</option><option value="6">{t('Every 6 hours')}</option><option value="12">{t('Every 12 hours')}</option><option value="24">{t('Every day')}</option><option value="72">{t('Every 3 days')}</option>
                </select></div>
            {/if}
          </div>
          <p class="hint">{t('Automatic runs Mihomo and bridges nodes that only Xray supports, such as XHTTP, through a local Xray instance.')}</p>
          {#if sel.source.kind === 'remote'}
            {@const src = sel.source}
            <div>
              <div class="row-sw"><div><div class="t">{t('Update through the proxy')}</div><div class="d">{t('Helps when the provider’s site is blocked. Used only while connected.')}</div></div>
                <button class="switch" role="switch" aria-label={t('Update through the proxy')} aria-checked={src.via_proxy} onclick={() => edit({ via_proxy: !src.via_proxy })}></button></div>
              <div class="row-sw"><div><div class="t">User-Agent</div><div class="d">{t('Some providers return a different format per app')}</div></div>
                <input class="input mono ua" value={src.user_agent ?? ''} placeholder={t('clash.meta (default)')} onchange={(e) => edit({ user_agent: e.currentTarget.value })} aria-label="User-Agent" /></div>
            </div>
          {/if}
          {#if sel.announce}<p class="announce"><b>{t('From the provider')}:</b> <span class="selectable">{sel.announce}</span></p>{/if}
          {#if sel.warnings.length}
            <details class="warn-box"><summary>{tn(sel.warnings.length, '{n} entry was skipped', '{n} entries were skipped')}</summary>
              <ul>{#each sel.warnings.slice(0, 20) as w, i (i)}<li class="mono">{w}</li>{/each}</ul></details>
          {/if}
          <div class="actions">
            {#if sel.id !== app.settings?.active_profile}
              <button class="btn primary" onclick={() => app.run(() => api.setActiveProfile(sel!.id), t('Now using {name}', { name: sel!.name }))}><Icon name="check" />{t('Use this profile')}</button>
            {/if}
            {#if sel.source.kind === 'remote'}
              <button class="btn" class:busy={updating.has(sel.id)} onclick={() => update(sel!.id)} disabled={updating.has(sel.id)}><Icon name="refresh" />{t('Update now')}</button>
              <button class="btn" onclick={() => sel?.source.kind === 'remote' && app.run(() => navigator.clipboard.writeText(sel.source.kind === 'remote' ? sel.source.url : ''), t('Subscription URL copied'))}><Icon name="copy" />{t('Copy URL')}</button>
            {/if}
            <button class="btn" onclick={() => (sharing = sel!)}><Icon name="qr" />{t('Share')}</button>
            <button class="btn" onclick={() => (editing = sel!)}><Icon name="edit" />{t('Edit config')}</button>
            {#if sel.support_url}<button class="btn ghost" onclick={() => app.run(() => openUrl(sel!.support_url!))}><Icon name="external" />{t('Provider')}</button>{/if}
            {#if confirmDelete}
              <span class="confirm">{t('Delete {name}?', { name: sel.name })} <button class="btn danger" onclick={remove}>{t('Delete')}</button><button class="btn ghost" onclick={() => (confirmDelete = false)}>{t('Keep')}</button></span>
            {:else}
              <button class="btn ghost danger" onclick={() => (confirmDelete = true)}><Icon name="trash" />{t('Delete')}</button>
            {/if}
          </div>
        </section>
      {/key}
    {/if}
  </div>
{/if}

{#if editing}<EditorModal profile={editing} onclose={() => (editing = null)} />{/if}
{#if sharing}<ShareModal profile={sharing} onclose={() => (sharing = null)} />{/if}

<style>
  .pf { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 16px; align-items: start; }
  @container (max-width: 820px) { .pf { grid-template-columns: minmax(0, 1fr); } }
  .plist { display: flex; flex-direction: column; gap: 10px; }
  .prof { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 12px; padding: 14px; border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); text-align: left; }
  .prof:hover { border-color: var(--line-2); }
  .prof[aria-current='true'] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .radio { width: 16px; height: 16px; border-radius: 50%; border: 1.5px solid var(--line-2); margin-top: 2px; display: grid; place-items: center; }
  .radio.on { border-color: var(--accent); }
  .radio.on::after { content: ''; width: 8px; height: 8px; border-radius: 50%; background: var(--accent); }
  .t { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .t b { font-weight: 600; }
  .m { grid-column: 2; display: flex; gap: 6px 14px; flex-wrap: wrap; font-size: 12px; color: var(--muted); }
  .u { grid-column: 2; display: flex; flex-direction: column; gap: 5px; margin-top: 6px; }
  .s { display: flex; justify-content: space-between; font-size: 12px; color: var(--muted); }
  .detail { display: flex; flex-direction: column; gap: 14px; }
  .detail .card-h { margin: 0; }
  .small { font-size: 12px; }
  .ua { width: 220px; font-size: 12px; }
  .announce { margin: 0; padding: 9px 11px; border-radius: 8px; background: var(--sunk); border: 1px solid var(--line); font-size: 13px; overflow-wrap: anywhere; }
  .warn-box { font-size: 12.5px; color: var(--warn); }
  .warn-box ul { margin: 8px 0 0; padding-left: 18px; color: var(--muted); font-size: 11.5px; }
  .confirm { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; }
</style>
