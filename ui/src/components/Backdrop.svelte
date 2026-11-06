<script lang="ts">
  import { app } from '../lib/state.svelte';
  import { pictureStyle } from '../lib/pictures';

  // The picture behind the window: the open tab's own, or the one behind every tab.
  let pic = $derived(app.backdrop);
  let url = $derived(app.pictureUrl(pic?.file));
  let shown = $derived(!!(pic && url));
  $effect(() => {
    document.documentElement.classList.toggle('look-has-bg', shown);
  });
</script>

{#if pic && url}
  <!-- Blur spreads past the edges; a little scale keeps them covered. -->
  <div class="backdrop" aria-hidden="true"
    style="{pictureStyle(pic, url)}; filter: blur({pic.blur ?? 0}px); transform: scale({pic.blur ? 1.05 : 1});"></div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 0; pointer-events: none; }
</style>
