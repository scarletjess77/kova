<script lang="ts">
  import type { Check } from '../lib/api';
  import { describe, fixLabel, runFix } from '../lib/checks';
  import Icon from './Icon.svelte';

  let { checks, onfixed, close = () => {} }: { checks: Check[]; onfixed?: () => void; close?: () => void } = $props();

  async function fix(f: string) {
    await runFix(f, close);
    onfixed?.();
  }
</script>

<ol class="checks">
  {#each checks as c (c.id)}
    {@const d = describe(c)}
    <li class={c.state}>
      <span class="mark" aria-hidden="true">{#if c.state === 'ok'}<Icon name="check" />{:else if c.state === 'skipped'}–{:else}<Icon name="alert" />{/if}</span>
      <div class="txt">
        <b>{d.title}</b>
        {#if d.text}<span>{d.text}</span>{/if}
      </div>
      {#if c.state !== 'ok' && c.state !== 'skipped' && c.fix && fixLabel(c.fix)}
        <button class="btn small-btn" onclick={() => fix(c.fix!)}>{fixLabel(c.fix)}</button>
      {/if}
    </li>
  {/each}
</ol>

<style>
  .checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  li { display: flex; gap: 12px; align-items: center; padding: 10px 0; border-top: 1px solid var(--line); }
  li:first-child { border-top: 0; }
  .mark { width: 24px; height: 24px; border-radius: 50%; display: grid; place-items: center; flex: none; font-weight: 600; color: var(--faint); background: var(--sunk); }
  .mark :global(.i) { width: 14px; height: 14px; stroke-width: 2.2; }
  .ok .mark { color: var(--ok); background: var(--ok-soft); }
  .warn .mark { color: var(--warn); background: var(--warn-soft); }
  .fail .mark { color: var(--bad); background: var(--bad-soft); }
  .skipped { opacity: .55; }
  .txt { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; font-size: 13px; }
  .txt span { color: var(--muted); overflow-wrap: anywhere; }
  .small-btn { height: 30px; font-size: 12.5px; flex: none; }
</style>
