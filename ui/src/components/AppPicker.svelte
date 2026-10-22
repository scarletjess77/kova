<script lang="ts">
  import { api, type VpnApp, type VpnAppsMode } from '../lib/api';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const saved = app.settings?.vpn_apps ?? { mode: 'all', apps: [] };
  let mode = $state<VpnAppsMode>(saved.mode);
  let chosen = $state(new Set(saved.apps));
  let apps = $state<VpnApp[] | null>(null);
  let query = $state('');
  let system = $state(false);
  let busy = $state(false);

  void app.run(() => api.vpnApps()).then((r) => (apps = r?.apps.sort((a, b) => a.label.localeCompare(b.label)) ?? []));

  // Chosen apps first, so they are easy to find again; system apps only on request.
  let shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return (apps ?? [])
      .filter((a) => (system || !a.system || chosen.has(a.package)) && (!q || a.label.toLowerCase().includes(q) || a.package.includes(q)))
      .sort((a, b) => Number(chosen.has(b.package)) - Number(chosen.has(a.package)));
  });

  function flip(pkg: string) {
    const next = new Set(chosen);
    if (!next.delete(pkg)) next.add(pkg);
    chosen = next;
  }

  async function save() {
    if (mode === 'only' && chosen.size === 0) {
      app.notify(t('Choose at least one app'));
      return;
    }
    busy = true;
    await app.saveSettings((s) => (s.vpn_apps = { mode, apps: [...chosen] }), t('Saved'));
    // The app list is part of the VPN interface itself, so a running VPN is reopened.
    if (app.status.running) {
      await app.run(() => api.disconnect());
      await app.run(() => api.connect());
    }
    busy = false;
    onclose();
  }
</script>

<Modal title={t('Apps through the VPN')} {onclose}>
  <div class="seg" role="group" aria-label={t('Apps through the VPN')}>
    <button aria-pressed={mode === 'all'} onclick={() => (mode = 'all')}>{t('All apps')}</button>
    <button aria-pressed={mode === 'only'} onclick={() => (mode = 'only')}>{t('Only chosen')}</button>
    <button aria-pressed={mode === 'except'} onclick={() => (mode = 'except')}>{t('All but chosen')}</button>
  </div>
  <p class="hint">
    {mode === 'all' ? t('Every app goes through the VPN.') : mode === 'only' ? t('Only the apps ticked below go through the VPN; the rest connect as if Kova were off.') : t('The apps ticked below skip the VPN; every other app goes through it.')}
  </p>

  {#if mode !== 'all'}
    <div class="tools">
      <div class="search"><Icon name="search" /><input class="input" placeholder={t('Search apps')} bind:value={query} /></div>
      <label class="sys"><input type="checkbox" bind:checked={system} />{t('System apps')}</label>
    </div>
    {#if apps === null}
      <p class="muted">{t('Loading the apps…')}</p>
    {:else}
      <div class="list">
        {#each shown as a (a.package)}
          <label class="row">
            <img src={a.icon} alt="" width="32" height="32" />
            <span class="nm"><span class="t">{a.label}</span><span class="p mono">{a.package}</span></span>
            <input type="checkbox" checked={chosen.has(a.package)} onchange={() => flip(a.package)} />
          </label>
        {:else}
          <p class="muted">{t('No apps match.')}</p>
        {/each}
      </div>
    {/if}
  {/if}

  {#snippet footer()}
    <span class="count muted">{mode === 'all' ? '' : tn(chosen.size, '{n} app chosen', '{n} apps chosen')}</span>
    <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
    <button class="btn primary" class:busy onclick={save} disabled={busy}>{app.status.running ? t('Save and reconnect') : t('Save')}</button>
  {/snippet}
</Modal>

<style>
  .hint { margin: 10px 0 0; font-size: 12.5px; color: var(--muted); }
  .tools { display: flex; gap: 12px; align-items: center; margin-top: 14px; flex-wrap: wrap; }
  .tools .search { flex: 1 1 200px; }
  .tools .search .input { width: 100%; }
  .sys { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--muted); }
  .list { margin-top: 10px; display: flex; flex-direction: column; max-height: 50vh; overflow: auto; border: 1px solid var(--line); border-radius: 8px; }
  .row { display: flex; align-items: center; gap: 12px; padding: 8px 12px; cursor: pointer; }
  .row + .row { border-top: 1px solid var(--line); }
  .row img { flex: none; border-radius: 8px; }
  .nm { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .nm .t { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nm .p { font-size: 11px; color: var(--faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row input { width: 18px; height: 18px; flex: none; accent-color: var(--accent); }
  .count { margin-right: auto; font-size: 12.5px; }
  .muted { padding: 8px 2px; }
</style>
