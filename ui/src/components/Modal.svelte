<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { title, wide = false, onclose, children, footer, badge }: {
    title: string; wide?: boolean; onclose: () => void; children: Snippet; footer?: Snippet; badge?: Snippet;
  } = $props();
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="modal" class:wide role="dialog" aria-modal="true" aria-label={title}>
    <header>
      <h3>{title}</h3>
      {@render badge?.()}
      <button class="x" onclick={onclose} aria-label="Close"><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgb(0 0 0 / .55); display: grid; place-items: center; padding: 16px; z-index: 20; }
  .modal { background: var(--win); border: 1px solid var(--line); border-radius: 14px; box-shadow: var(--shadow); width: min(580px, 100%); max-height: calc(100% - 32px); overflow: auto; display: flex; flex-direction: column; }
  .modal.wide { width: min(860px, 100%); }
  header { display: flex; align-items: center; gap: 10px; padding: 16px 18px; border-bottom: 1px solid var(--line); }
  h3 { margin: 0; font: 600 16px var(--f-ui); }
  header .x { margin-left: auto; }
  .body { padding: 18px; display: flex; flex-direction: column; gap: 14px; }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 14px 18px; border-top: 1px solid var(--line); }
</style>
