<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import { PRESETS, presetOf, type Preset } from '../lib/layout';
  import Icon from './Icon.svelte';

  let PRESET_NAME = $derived<Record<Preset, string>>({ standard: t('Standard'), simple: t('Simple'), everything: t('Everything') });
  let current = $derived(presetOf(app.layout, app.form));
</script>

<!-- Shown while arranging: what to do, the presets, and the way out. -->
<div class="lbar" role="region" aria-label={t('Edit layout')}>
  <div class="txt">
    <b>{app.form === 'phone' ? t('Editing the phone layout') : t('Editing the desktop layout')}</b>
    <span>{t('Drag the handle to move a tab or card, the eye to show or hide it. Changes save as you go.')}</span>
  </div>
  <div class="seg" role="group" aria-label={t('Layout preset')}>
    {#each PRESETS as p (p)}<button aria-pressed={current === p} onclick={() => app.applyPreset(p)}>{PRESET_NAME[p]}</button>{/each}
  </div>
  <button class="btn primary" onclick={() => (app.editingLayout = false)}><Icon name="check" />{t('Done')}</button>
</div>

<style>
  /* Sticks to the bottom of the page: over the content while there is more below it, in
     place at the end, so showing it never moves what is on the page. */
  .lbar { position: sticky; bottom: 12px; z-index: 5; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin: 16px 0 0; padding: 12px 16px; border: 1px solid var(--accent); border-radius: var(--r); background: linear-gradient(var(--accent-soft), var(--accent-soft)), var(--win); box-shadow: var(--shadow); }
  .txt { flex: 1 1 280px; min-width: 0; display: flex; flex-direction: column; gap: 2px; font-size: 13px; }
  .txt span { color: var(--muted); }
</style>
