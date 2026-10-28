<script lang="ts">
  import { latClass } from '../lib/format';
  import { t } from '../lib/i18n.svelte';

  let { delay, testing = false, dead = false }: { delay: number | null | undefined; testing?: boolean; dead?: boolean } = $props();
</script>

{#if testing}
  <span class="lat testing">…</span>
{:else if delay == null && dead}
  <!-- In the Night sky theme a server that didn't answer is a fallen star. -->
  <span class="lat none" title={t('No answer')}><svg class="fallen" viewBox="0 0 16 12" shape-rendering="crispEdges" aria-hidden="true"><path class="tail" d="M5 7h2v1H5zM7 6h2v1H7zM9 5h2v1H9zM11 4h2v1h-2zM13 3h2v1h-2z" /><path d="M3 8h1v1H3zM2 9h3v1H2zM3 10h1v1H3z" /></svg><span class="dash">—</span></span>
{:else if delay == null}
  <span class="lat none">—</span>
{:else}
  <span class="lat {latClass(delay)}">{t('{n} ms', { n: delay })}</span>
{/if}

<style>
  .fallen { display: none; width: 20px; height: 15px; fill: currentColor; vertical-align: -3px; }
  .fallen .tail { opacity: .45; }
  :global(:root[data-theme='night']) .fallen { display: inline-block; }
  :global(:root[data-theme='night']) .fallen + .dash { display: none; }
</style>
