<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Donate from './Donate.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let donateOpen = $state(false);

  function toggleTheme() {
    const light = document.documentElement.dataset.theme === 'light';
    void app.saveSettings((s) => (s.theme = light ? 'dark' : 'light'));
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="2" width="20" height="20" rx="6" fill="var(--accent)" /><path d="M8.5 7v10M15.5 7l-5.2 5 5.2 5" stroke="var(--accent-fg)" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
    Kova
  </div>
  {#if app.activeProfile}
    <div class="tb-profile" data-tauri-drag-region>/ <b>{app.activeProfile.name}</b></div>
  {/if}
  <div class="spacer" data-tauri-drag-region></div>
  <button class="tb-btn donate" onclick={() => (donateOpen = true)} title={t('Donate')} aria-label={t('Donate')}><Icon name="heart" /><span>{t('Donate')}</span></button>
  <button class="tb-btn" onclick={toggleTheme} title={t('Switch theme')} aria-label={t('Switch theme')}><Icon name="moon" /></button>
  <div class="winctl">
    <button onclick={() => api.windowAction('minimize')} aria-label={t('Minimize')}><Icon name="min" /></button>
    <button onclick={() => api.windowAction('maximize')} aria-label={t('Maximize')}><Icon name="max" /></button>
    <button class="close" onclick={() => api.windowAction('hide')} aria-label={t('Close to tray')} title={t('Close to tray')}><Icon name="x" /></button>
  </div>
</header>

{#if donateOpen}
  <Modal title={t('Support Kova')} onclose={() => (donateOpen = false)}>
    <Donate />
  </Modal>
{/if}

<style>
  .titlebar { grid-column: 1 / -1; display: flex; align-items: center; gap: 12px; padding-left: 14px; border-bottom: 1px solid var(--line); background: var(--win); }
  .brand { display: flex; align-items: center; gap: 9px; font-family: var(--f-display); font-weight: 700; font-size: 13px; letter-spacing: .02em; }
  .brand svg { width: 20px; height: 20px; }
  .tb-profile { color: var(--muted); font-size: 12.5px; display: flex; gap: 8px; min-width: 0; }
  .tb-profile b { font-weight: 500; color: var(--fg); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .spacer { flex: 1; align-self: stretch; }
  .tb-btn { height: 28px; width: 30px; display: grid; place-items: center; border: 0; background: none; border-radius: 6px; color: var(--muted); }
  .tb-btn:hover { background: var(--sunk); color: var(--fg); }
  .tb-btn.donate { width: auto; gap: 6px; padding: 0 9px; display: flex; align-items: center; font-size: 12.5px; font-weight: 500; }
  .tb-btn.donate :global(.i) { width: 15px; height: 15px; }
  .winctl { display: flex; height: 100%; }
  .winctl button { width: 46px; display: grid; place-items: center; color: var(--muted); border: 0; background: none; }
  .winctl button:hover { background: var(--sunk); }
  .winctl .close:hover { background: #c42b1c; color: #fff; }
  .winctl :global(.i) { width: 14px; height: 14px; stroke-width: 1.4; }
</style>
