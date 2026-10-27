<script lang="ts">
  import { api, type Profile } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { profile, onclose }: { profile: Profile; onclose: () => void } = $props();

  let text = $state('');
  let original = $state('');
  let error = $state<string | null>(null);
  let busy = $state(false);

  $effect(() => {
    api.getProfileContent(profile.id).then((t) => { text = t; original = t; }).catch((e) => (error = String(e)));
  });

  const file = $derived(profile.format === 'clash' ? 'config.yaml' : profile.format === 'sing-box' || profile.format === 'xray' ? 'config.json' : 'links.txt');

  async function save() {
    busy = true;
    error = null;
    try {
      await api.saveProfileContent(profile.id, text);
      app.notify(profile.source.kind === 'remote' ? t('Saved. The next subscription update will replace your edits.') : t('Config is valid and saved'));
      onclose();
    } catch (e) {
      error = String(e);
    }
    busy = false;
  }
</script>

<Modal title={profile.name} wide {onclose}>
  {#snippet badge()}<span class="chip mono">{file}</span>{/snippet}
  <textarea class="code" spellcheck="false" bind:value={text} aria-label={t('Profile configuration')}></textarea>
  {#if error}
    <div class="err"><Icon name="alert" /><span class="selectable">{error}</span></div>
  {:else}
    <p class="hint">{t('Kova checks the config with Mihomo before saving. Ports, controller and TUN are managed by Kova and override these values.')}</p>
  {/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
    <button class="btn primary" class:busy onclick={save} disabled={busy || text === original}>{#if busy}<Icon name="refresh" />{t('Checking…')}{:else}{t('Check and save')}{/if}</button>
  {/snippet}
</Modal>

<style>
  .code { font: 12px/1.6 var(--f-mono); background: var(--sunk); border: 1px solid var(--line); border-radius: 8px; padding: 12px; width: 100%; min-height: 420px; resize: vertical; white-space: pre; tab-size: 2; color: var(--fg); }
  .code:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  .err { display: flex; gap: 8px; align-items: flex-start; color: var(--bad); background: var(--bad-soft); padding: 10px 12px; border-radius: 8px; font-size: 12.5px; }
</style>
