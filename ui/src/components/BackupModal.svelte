<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import { api, type BackupSummary } from '../lib/api';
  import { i18n, t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  // Saving or restoring a backup: every profile with its content, and the settings.
  // On the desktop the system's file dialog picks the file; on Android its file picker.
  let { mode, onclose }: { mode: 'save' | 'restore'; onclose: () => void } = $props();

  let password = $state('');
  let again = $state('');
  let busy = $state(false);
  let text = $state<string | null>(null);
  let summary = $state<BackupSummary | null>(null);
  let withSettings = $state(true);

  const fileName = () => `kova-backup-${new Date().toISOString().slice(0, 10)}.json`;
  let mismatch = $derived(password !== again);

  async function saveBackup() {
    if (mismatch) return;
    busy = true;
    try {
      const name = fileName();
      let path: string | null = null;
      if (!app.mobile) {
        path = await save({ defaultPath: name, filters: [{ name: t('Kova backup'), extensions: ['json'] }] });
        if (!path) return;
      }
      const saved = await app.run(() => api.backupSave(password || null, path, name));
      if (saved) {
        app.notify(t('Backup saved'));
        onclose();
      }
    } finally {
      busy = false;
    }
  }

  async function chooseFile() {
    if (app.mobile) {
      text = (await app.run(() => api.pickTextFile())) ?? null;
    } else {
      const path = await open({ multiple: false, filters: [{ name: t('Kova backup'), extensions: ['json'] }] });
      if (path) text = (await app.run(() => invoke<string>('read_text_file', { path }))) ?? null;
    }
    if (text) await openBackup();
  }

  async function openBackup() {
    if (!text) return;
    busy = true;
    summary = (await app.run(() => api.backupOpen(text!, password || null))) ?? null;
    busy = false;
  }

  async function restore() {
    busy = true;
    const r = await app.run(() => api.backupRestore(withSettings));
    busy = false;
    if (!r) return;
    const st = await api.getState().catch(() => null);
    if (st) {
      app.profiles = st.profiles;
      app.settings = st.settings;
      app.applyTheme();
    }
    app.notify(r.skipped
      ? `${tn(r.added, 'Restored {n} profile', 'Restored {n} profiles')} · ${tn(r.skipped, '{n} profile was already here', '{n} profiles were already here')}`
      : tn(r.added, 'Restored {n} profile', 'Restored {n} profiles'));
    onclose();
  }

  const made = (s: BackupSummary) =>
    t('Made by Kova {version} on {date}', { version: s.kova ?? '?', date: new Date((s.created ?? 0) * 1000).toLocaleDateString(i18n.lang) });
</script>

{#if mode === 'save'}
  <Modal title={t('Save a backup')} {onclose}>
    <p class="note"><Icon name="alert" />{t('The file holds your subscriptions: anyone who has it can use them. A password encrypts it.')}</p>
    <div class="fld">
      <label for="bk-pw">{t('Password (optional)')}</label>
      <input id="bk-pw" class="input" type="password" autocomplete="new-password" bind:value={password} />
    </div>
    {#if password}
      <div class="fld">
        <label for="bk-pw2">{t('Password again')}</label>
        <input id="bk-pw2" class="input" type="password" autocomplete="new-password" bind:value={again} />
        {#if again && mismatch}<span class="bad">{t('The passwords don’t match')}</span>{/if}
      </div>
    {/if}
    {#snippet footer()}
      <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
      <button class="btn primary" class:busy onclick={saveBackup} disabled={busy || mismatch}>{#if busy}<Icon name="refresh" />{t('Working…')}{:else}{t('Save backup')}{/if}</button>
    {/snippet}
  </Modal>
{:else}
  <Modal title={t('Restore a backup')} {onclose}>
    {#if !text}
      <div class="drop">
        <Icon name="file" />
        <b>{t('Choose the backup file')}</b>
        <button class="btn" onclick={chooseFile}>{t('Choose a file…')}</button>
      </div>
    {:else if summary?.needs_password}
      <p>{t('This backup is encrypted. Enter its password.')}</p>
      <div class="fld">
        <label for="bk-open">{t('Password')}</label>
        <!-- svelte-ignore a11y_autofocus -->
        <input id="bk-open" class="input" type="password" autocomplete="current-password" autofocus bind:value={password}
          onkeydown={(e) => e.key === 'Enter' && openBackup()} />
      </div>
    {:else if summary}
      <p class="muted">{made(summary)}</p>
      <div>
        <b>{tn(summary.profiles.length, '{n} profile', '{n} profiles')}</b>
        <ul class="names">{#each summary.profiles as name, i (i)}<li class="selectable">{name}</li>{/each}</ul>
      </div>
      <label class="chk"><input type="checkbox" bind:checked={withSettings} />{t('Also restore the settings: theme, rules, DNS and the rest')}</label>
    {/if}
    {#snippet footer()}
      <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
      {#if summary?.needs_password}
        <button class="btn primary" class:busy onclick={openBackup} disabled={busy || !password}>{#if busy}<Icon name="refresh" />{t('Working…')}{:else}{t('Open')}{/if}</button>
      {:else if summary}
        <button class="btn primary" class:busy onclick={restore} disabled={busy}>{#if busy}<Icon name="refresh" />{t('Working…')}{:else}{t('Restore')}{/if}</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .note { display: flex; gap: 8px; align-items: flex-start; margin: 0; padding: 10px 12px; border-radius: var(--r); background: var(--warn-soft); color: var(--warn); font-size: 13px; }
  .note :global(.i) { width: 16px; height: 16px; flex: none; margin-top: 1px; }
  .bad { color: var(--bad); font-size: 12px; }
  .drop { border: 1.5px dashed var(--line-2); border-radius: var(--r); padding: 24px; text-align: center; color: var(--muted); display: flex; flex-direction: column; gap: 8px; align-items: center; }
  .drop :global(.i) { width: 26px; height: 26px; color: var(--faint); }
  .drop b { color: var(--fg); font-weight: 600; }
  .names { margin: 6px 0 0; padding-inline-start: 18px; max-height: 160px; overflow: auto; color: var(--muted); font-size: 13px; }
  .chk { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; }
  .muted { color: var(--muted); margin: 0; }
</style>
