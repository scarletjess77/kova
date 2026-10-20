<script lang="ts">
  import { readText } from '@tauri-apps/plugin-clipboard-manager';
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { api } from '../lib/api';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { onclose }: { onclose: () => void } = $props();

  type Tab = 'link' | 'clip' | 'file';
  let tab = $state<Tab>('link');
  let input = $state('');
  let name = $state('');
  let busy = $state(false);
  let clip = $state<string | null>(null);
  let file = $state<{ path: string; text: string } | null>(null);

  const SCHEMES = /^(vless|vmess|trojan|ss|hysteria2|hy2|tuic|wireguard|wg|anytls|socks5?):\/\//im;
  let clipLinks = $derived(clip ? clip.split(/\s+/).filter((l) => SCHEMES.test(l)).length : 0);
  let clipIsUrl = $derived(!!clip && /^https?:\/\/\S+$/.test(clip.trim()));

  async function loadClipboard() {
    try { clip = (await readText()) ?? ''; } catch { clip = ''; }
  }
  $effect(() => { if (tab === 'clip' && clip === null) void loadClipboard(); });

  async function pickFile() {
    const path = await open({ multiple: false, filters: [{ name: t('Configs and link lists'), extensions: ['yaml', 'yml', 'json', 'txt', 'conf'] }] });
    if (!path) return;
    const text = await app.run(() => invoke<string>('read_text_file', { path }));
    if (text != null) file = { path: String(path), text };
  }

  let payload = $derived(tab === 'link' ? input.trim() : tab === 'clip' ? (clip ?? '').trim() : file?.text ?? '');
  let fileName = $derived(file ? file.path.split(/[\\/]/).pop()?.replace(/\.\w+$/, '') ?? '' : '');

  async function submit() {
    if (!payload) return;
    busy = true;
    const p = await app.run(() => api.addProfile(payload, name || (tab === 'file' ? fileName : undefined)), (p) => tn(p.node_count, 'Added {name} · {n} node', 'Added {name} · {n} nodes', { name: p.name }));
    busy = false;
    if (p) onclose();
  }
</script>

<Modal title={t('Add profile')} {onclose}>
  <div class="seg" role="group" aria-label={t('Import from')}>
    <button aria-pressed={tab === 'link'} onclick={() => (tab = 'link')}>{t('Link')}</button>
    <button aria-pressed={tab === 'clip'} onclick={() => (tab = 'clip')}>{t('Clipboard')}</button>
    <button aria-pressed={tab === 'file'} onclick={() => (tab = 'file')}>{t('File')}</button>
  </div>

  {#if tab === 'link'}
    <div class="fld">
      <label for="imp-url">{t('Subscription URL or share links')}</label>
      <textarea id="imp-url" class="input mono" rows="4" bind:value={input} placeholder={t('https://…\nor vless://…  hy2://…  one per line')}></textarea>
    </div>
  {:else if tab === 'clip'}
    <div class="drop">
      <Icon name="clip" />
      {#if clip === null}
        <span>{t('Reading the clipboard…')}</span>
      {:else if clipIsUrl}
        <b>{t('A subscription URL is in your clipboard')}</b>
        <span class="mono small">{clip.trim().slice(0, 80)}</span>
      {:else if clipLinks}
        <b>{tn(clipLinks, '{n} share link found in your clipboard', '{n} share links found in your clipboard')}</b>
      {:else if clip.trim()}
        <b>{t('The clipboard has text')}</b><span>{t('Kova will try to read it as a config.')}</span>
      {:else}
        <b>{t('The clipboard is empty')}</b><span>{t('Copy a subscription URL or share links, then check again.')}</span>
      {/if}
      <button class="btn ghost" onclick={loadClipboard}><Icon name="refresh" />{t('Check again')}</button>
    </div>
  {:else}
    <div class="drop">
      <Icon name="file" />
      {#if file}
        <b>{file.path.split(/[\\/]/).pop()}</b><span>{(file.text.length / 1024).toFixed(1)} KB</span>
      {:else}
        <b>{t('Choose a config file')}</b><span>{t('Clash / Mihomo YAML, sing-box JSON, or a text file of share links')}</span>
      {/if}
      <button class="btn" onclick={pickFile}>{t('Browse…')}</button>
    </div>
  {/if}

  <div class="fld">
    <label for="imp-name">{t('Name')} <span class="muted">{t('(optional)')}</span></label>
    <input id="imp-name" class="input" bind:value={name} placeholder={t('Taken from the subscription')} />
  </div>

  <div>
    <div class="eyebrow">{t('Understands')}</div>
    <div class="fmt">
      {#each ['Clash / Mihomo YAML', 'sing-box JSON', t('Base64 subscriptions'), 'vless://', 'vmess://', 'trojan://', 'ss://', 'hy2://', 'tuic://', 'anytls://', 'wireguard://'] as f (f)}
        <span class="chip" class:mono={f.endsWith('//')}>{f}</span>
      {/each}
    </div>
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
    <button class="btn primary" class:busy onclick={submit} disabled={!payload || busy}>{#if busy}<Icon name="refresh" />{t('Importing…')}{:else}{t('Import')}{/if}</button>
  {/snippet}
</Modal>

<style>
  .drop { border: 1.5px dashed var(--line-2); border-radius: var(--r); padding: 24px; text-align: center; color: var(--muted); display: flex; flex-direction: column; gap: 8px; align-items: center; }
  .drop :global(.i) { width: 26px; height: 26px; color: var(--faint); }
  .drop b { color: var(--fg); font-weight: 600; }
  .small { font-size: 12px; word-break: break-all; }
  .fmt { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
  textarea { font-size: 12.5px; }
</style>
