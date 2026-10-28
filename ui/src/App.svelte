<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from './lib/i18n.svelte';
  import { app } from './lib/state.svelte';
  import { resolveTheme } from './lib/themes';
  import ImportModal from './components/ImportModal.svelte';
  import NightSky from './components/NightSky.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Titlebar from './components/Titlebar.svelte';
  import Connections from './views/Connections.svelte';
  import Dashboard from './views/Dashboard.svelte';
  import Logs from './views/Logs.svelte';
  import Profiles from './views/Profiles.svelte';
  import Proxies from './views/Proxies.svelte';
  import Rules from './views/Rules.svelte';
  import SiteCheck from './views/SiteCheck.svelte';
  import SettingsView from './views/Settings.svelte';

  let failed = $state<string | null>(null);
  let night = $derived(resolveTheme(app.settings?.theme).id === 'night');
  onMount(() => { app.init().catch((e) => (failed = String(e))); });

  // Ctrl+V anywhere outside a text field imports from the clipboard.
  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (e.ctrlKey && e.key.toLowerCase() === 'v' && !['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName)) {
      app.importOpen = true;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if night}<NightSky />{/if}
<div class="app" class:mobile={app.mobile} class:tv={app.tv}>
  <Titlebar />
  <Sidebar />
  <main class="main">
    {#if failed}
      <div class="empty"><b>{t('Kova could not start')}</b><span class="selectable">{failed}</span></div>
    {:else if app.ready}
      {#if app.view === 'dashboard'}<Dashboard />
      {:else if app.view === 'proxies'}<Proxies />
      {:else if app.view === 'profiles'}<Profiles />
      {:else if app.view === 'connections'}<Connections />
      {:else if app.view === 'rules'}<Rules />
      {:else if app.view === 'check'}<SiteCheck />
      {:else if app.view === 'logs'}<Logs />
      {:else}<SettingsView />{/if}
    {/if}
  </main>
  {#if !app.mobile}<StatusBar />{/if}
</div>

{#if app.importOpen}<ImportModal onclose={() => (app.importOpen = false)} />{/if}
{#if app.toast}<div class="toast" role="status">{app.toast}</div>{/if}

<style>
  .toast { position: fixed; left: 50%; bottom: 44px; transform: translateX(-50%); background: var(--fg); color: var(--win); padding: 9px 14px; border-radius: 9px; font-size: 13px; box-shadow: var(--shadow); z-index: 30; max-width: min(640px, calc(100% - 32px)); }
</style>
