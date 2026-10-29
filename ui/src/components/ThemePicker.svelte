<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import { THEMES, type ThemeId } from '../lib/themes';

  let current = $derived(app.settings?.theme);
  const pick = (id: ThemeId) => app.saveSettings((x) => (x.theme = id));
</script>

<!-- Each preview carries its own data-theme, so it shows that theme's colours. -->
<div class="themes" role="radiogroup" aria-label={t('Theme')}>
  <button class="th" role="radio" aria-checked={current === 'system'} onclick={() => pick('system')}>
    <span class="sw split"><span data-theme="dark"><i></i></span><span data-theme="light"><i></i></span></span>{t('Match system')}
  </button>
  {#each THEMES as th (th.id)}
    <button class="th" role="radio" aria-checked={current === th.id} onclick={() => pick(th.id)}>
      <span class="sw" data-theme={th.id}><i></i><b></b></span>{th.label()}
    </button>
  {/each}
</div>

<style>
  .themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(118px, 1fr)); gap: 8px; }
  .th { display: flex; flex-direction: column; gap: 7px; padding: 6px 6px 8px; border: 1px solid var(--line); border-radius: 10px; background: var(--sunk); font-size: 12.5px; font-weight: 500; color: var(--muted); text-align: center; }
  .th[aria-checked='true'] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); color: var(--fg); }
  /* A tiny window: the theme's background, a card, and its accent. */
  .sw { position: relative; display: block; height: 46px; border-radius: 6px; background: var(--win); border: 1px solid var(--line); overflow: hidden; }
  .sw i { position: absolute; left: 8px; right: 30px; top: 9px; bottom: 9px; border-radius: 4px; background: var(--panel); border: 1px solid var(--line); }
  .sw b { position: absolute; right: 8px; top: 50%; width: 14px; height: 14px; margin-top: -7px; border-radius: 50%; background: var(--accent); }
  /* A few pixel stars over the Night sky preview. */
  .sw[data-theme='night']::after { content: ''; position: absolute; left: 12px; top: 7px; width: 2px; height: 2px; background: #fff; box-shadow: 24px 9px #fff, 47px 2px #bdbdc8, 66px 27px #fff, 9px 29px #7c7c88, 86px 5px #fff, 35px 33px #bdbdc8; }
  .sw.split { display: flex; }
  .sw.split > span { position: relative; flex: 1; background: var(--win); }
  .sw.split i { right: 8px; }
</style>
