<script lang="ts">
  import { save as saveDialog } from '@tauri-apps/plugin-dialog';
  import { errText } from '../lib/api';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import { removePicture, type SlotId } from '../lib/pictures';
  import { THEMES } from '../lib/themes';
  import { THEME_FILE_LIMIT, buildThemeFile, keepThemePictures, readThemeFile, saveTextDocument, type ThemeFile } from '../lib/themefile';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { mode, onclose }: { mode: 'export' | 'import'; onclose: () => void } = $props();

  let busy = $state(false);
  let look = $derived(app.settings?.look ?? {});
  let pictureCount = $derived(Object.keys(look.pictures ?? {}).length);

  // ---- export ----
  let name = $state(t('My Kova theme'));
  let withLayout = $state(true);
  let withPictures = $state(true);
  let withCss = $state(true);

  async function exportTheme() {
    if (!app.settings) return;
    busy = true;
    try {
      const slug = name.trim().toLowerCase().replace(/[^a-z0-9а-яё]+/gi, '-').replace(/^-+|-+$/g, '') || 'kova-theme';
      const fileName = `${slug}.kovatheme`;
      let path: string | null = null;
      if (!app.mobile) {
        path = await saveDialog({ defaultPath: fileName, filters: [{ name: t('Kova theme'), extensions: ['kovatheme'] }] });
        if (!path) return;
      }
      const text = await buildThemeFile(app.settings, name, { layout: withLayout, pictures: withPictures, css: withCss });
      const saved = await app.run(() => saveTextDocument(path, fileName, text));
      if (saved) {
        app.notify(t('Theme saved'));
        onclose();
      }
    } catch (e) {
      app.notify(errText(e));
    } finally {
      busy = false;
    }
  }

  // ---- import ----
  let file = $state<ThemeFile | null>(null);
  let problem = $state<string | null>(null);
  let useLayout = $state(true);
  let useCss = $state(true);
  let input = $state<HTMLInputElement>();

  async function choose(f: File | undefined) {
    if (!f) return;
    problem = null;
    file = null;
    if (f.size > THEME_FILE_LIMIT) { problem = t('That file is too large for a theme'); return; }
    try {
      file = readThemeFile(await f.text());
    } catch (e) {
      const code = e instanceof Error ? e.message : '';
      problem = code === 'newer' ? t('This theme was made by a newer Kova. Update Kova to use it.') : t('This isn’t a Kova theme file.');
    }
  }

  let themeName = $derived(file?.theme ? (file.theme === 'system' ? t('Match system') : THEMES.find((x) => x.id === file!.theme)?.label() ?? file.theme) : null);
  let newPictures = $derived(Object.keys(file?.pictures ?? {}).length);

  async function applyTheme() {
    if (!file || !app.settings) return;
    busy = true;
    try {
      const oldSlots = Object.keys(app.settings.look?.pictures ?? {}) as SlotId[];
      const oldCss = app.settings.look?.css;
      const next = await keepThemePictures(file);
      // Pictures the new look doesn't use leave Kova's folder.
      for (const slot of oldSlots) if (!next.pictures?.[slot]) await removePicture(slot).catch(() => {});
      const css = useCss ? next.css : undefined;
      delete next.css;
      // Without the theme's CSS, the user's own stays.
      if (!css && oldCss) next.css = oldCss;
      const theme = file.theme;
      const layout = useLayout ? file.layout : undefined;
      await app.saveSettings((x) => {
        if (theme) x.theme = theme;
        x.look = next;
        if (layout) x.layout = { ...(x.layout ?? {}), ...layout };
      }, t('Theme applied'));
      // A theme's CSS is tried like one's own: kept on request, undone otherwise.
      if (css) app.tryCss(css);
      onclose();
    } catch (e) {
      app.notify(errText(e));
    } finally {
      busy = false;
    }
  }
</script>

{#if mode === 'export'}
  <Modal title={t('Export theme')} {onclose}>
    <p class="muted">{t('One file with the colour theme, the look, the layouts and the pictures, to keep or to share. Profiles and settings stay out.')}</p>
    <div class="fld"><label for="th-name">{t('Name')}</label><input id="th-name" class="input" bind:value={name} maxlength="80" /></div>
    <div class="checks">
      <label><input type="checkbox" bind:checked={withLayout} />{t('Layouts for phone and computer')}</label>
      <label class:dim={!pictureCount}><input type="checkbox" bind:checked={withPictures} disabled={!pictureCount} />{tn(pictureCount, '{n} picture', '{n} pictures')}</label>
      <label class:dim={!look.css}><input type="checkbox" bind:checked={withCss} disabled={!look.css} />{t('Custom CSS')}</label>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
      <button class="btn primary" class:busy disabled={busy} onclick={exportTheme}><Icon name="down" />{t('Save…')}</button>
    {/snippet}
  </Modal>
{:else}
  <Modal title={t('Import theme')} {onclose}>
    <p class="muted">{t('Replaces the look with the theme’s. Profiles and settings stay as they are.')}</p>
    <input type="file" accept=".kovatheme,application/json" hidden bind:this={input} onchange={(e) => choose(e.currentTarget.files?.[0])} />
    <div><button class="btn" onclick={() => input?.click()}><Icon name="file" />{file ? t('Choose another file') : t('Choose a theme file')}</button></div>
    {#if problem}<p class="bad-t">{problem}</p>{/if}
    {#if file}
      <div class="sum">
        <b>{file.name}</b>
        <ul>
          {#if themeName}<li>{t('Colour theme: {name}', { name: themeName })}</li>{/if}
          {#if file.look?.colors && Object.keys(file.look.colors).length}<li class="sw">{t('Own colours')} {#each Object.values(file.look.colors) as c, i (i)}<i style="background:{c}"></i>{/each}</li>{/if}
          {#if newPictures}<li>{tn(newPictures, '{n} picture', '{n} pictures')}</li>{/if}
          {#if file.layout}<li>{t('Layouts for phone and computer')}</li>{/if}
          {#if file.look?.css}<li>{t('Custom CSS')}</li>{/if}
        </ul>
      </div>
      <div class="checks">
        {#if file.layout}<label><input type="checkbox" bind:checked={useLayout} />{t('Use its layouts')}</label>{/if}
        {#if file.look?.css}<label><input type="checkbox" bind:checked={useCss} />{t('Use its custom CSS (asks to keep it, like your own)')}</label>{/if}
      </div>
    {/if}
    {#snippet footer()}
      <button class="btn ghost" onclick={onclose}>{t('Cancel')}</button>
      <button class="btn primary" class:busy disabled={busy || !file} onclick={applyTheme}><Icon name="check" />{t('Apply theme')}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .checks { display: flex; flex-direction: column; gap: 4px; }
  .checks label { display: flex; align-items: center; gap: 10px; padding: 6px 0; font-size: 13px; cursor: pointer; }
  .checks label.dim { opacity: .5; }
  .sum { border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); padding: 12px 14px; display: flex; flex-direction: column; gap: 6px; }
  .sum ul { margin: 0; padding-inline-start: 18px; color: var(--muted); font-size: 13px; display: flex; flex-direction: column; gap: 2px; }
  .sw i { display: inline-block; width: 12px; height: 12px; border-radius: 3px; margin-inline-start: 4px; vertical-align: -1px; box-shadow: inset 0 0 0 1px var(--line-2); }
  .bad-t { color: var(--bad); margin: 0; font-size: 13px; }
  .muted { margin: 0; }
</style>
