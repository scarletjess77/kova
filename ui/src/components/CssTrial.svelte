<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';

  // Inline !important rules: the CSS being tried can't hide this bar. Should it manage
  // to anyway, the countdown undoes it.
  const KEEP_VISIBLE = 'display:flex !important; visibility:visible !important; opacity:1 !important; position:fixed !important; z-index:2147483647 !important; left:50% !important; bottom:24px !important; transform:translateX(-50%) !important;';
</script>

{#if app.cssTrial}
  <div class="trial" role="alertdialog" aria-label={t('Keep the custom CSS?')} style={KEEP_VISIBLE}>
    <span>{t('Keep the custom CSS? It is undone in {n} s.', { n: app.cssTrial.left })}</span>
    <button class="btn primary" onclick={() => app.keepCss()}>{t('Keep')}</button>
    <button class="btn" onclick={() => app.undoCss()}>{t('Undo')}</button>
  </div>
{/if}

<style>
  .trial { align-items: center; gap: 10px; padding: 10px 12px; border-radius: 12px; background: #17171a; color: #ededef; border: 1px solid #3a3a40; box-shadow: 0 18px 50px rgb(0 0 0 / .5); font: 13px system-ui, sans-serif; max-width: calc(100% - 32px); flex-wrap: wrap; }
</style>
