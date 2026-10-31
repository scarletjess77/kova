<script lang="ts" module>
  // Which cards are folded, by id, kept on this device.
  const KEY = 'kova.folded';
  const load = (): Record<string, boolean> => {
    try { return JSON.parse(localStorage.getItem(KEY) ?? '{}') ?? {}; } catch { return {}; }
  };
  const folded = $state<Record<string, boolean>>(load());
  function remember() {
    try { localStorage.setItem(KEY, JSON.stringify(folded)); } catch { /* back open next time */ }
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  /** A settings card whose body folds away under its title; the choice is remembered. */
  let { id, title, closed = false, anchor, aside, children }: {
    id: string; title: string; closed?: boolean; anchor?: string; aside?: Snippet; children: Snippet;
  } = $props();

  let open = $derived(!(folded[id] ?? closed));
  const body = `fold-${Math.random().toString(36).slice(2)}`;
  function flip() {
    folded[id] = open;
    remember();
  }
</script>

<section class="card" class:closed={!open} id={anchor}>
  <div class="card-h">
    <button class="fold" aria-expanded={open} aria-controls={body} onclick={flip}><Icon name="chevron" /><h2>{title}</h2></button>
    {@render aside?.()}
  </div>
  {#if open}<div id={body}>{@render children()}</div>{/if}
</section>

<style>
  .fold { display: flex; align-items: center; gap: 6px; border: 0; background: none; padding: 2px 4px 2px 0; margin-inline-start: -4px; color: inherit; cursor: pointer; border-radius: 6px; min-width: 0; text-align: start; }
  .fold :global(.i) { width: 14px; height: 14px; color: var(--muted); transition: transform .15s; flex: none; }
  .fold[aria-expanded='true'] :global(.i) { transform: rotate(90deg); }
  .fold[aria-expanded='false'] :global(.i):dir(rtl) { transform: scaleX(-1); }
  @media (hover: hover) { .fold:hover :global(.i) { color: var(--fg); } }
  .closed .card-h { margin-bottom: 0; }
</style>
