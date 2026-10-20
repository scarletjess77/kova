<script lang="ts">
  import { latClass } from '../lib/format';

  let { delay, testing = false }: { delay: number | null | undefined; testing?: boolean } = $props();
</script>

{#if testing}
  <span class="lat testing">…</span>
{:else if delay == null}
  <span class="lat none">—</span>
{:else}
  <span class="lat {latClass(delay)}">{delay} ms</span>
{/if}
