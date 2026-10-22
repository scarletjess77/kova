<script lang="ts">
  import { api, type Check } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import CheckList from './CheckList.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { onclose }: { onclose: () => void } = $props();

  let checks = $state<Check[] | null>(null);
  let busy = $state(false);

  async function run() {
    busy = true;
    checks = (await app.run(() => api.diagnose())) ?? checks;
    busy = false;
  }
  void run();

  let verdict = $derived.by(() => {
    if (!checks) return '';
    const bad = checks.find((c) => c.state === 'fail') ?? checks.find((c) => c.state === 'warn');
    return bad ? t('Found it: see the step marked below.') : t('Everything checks out. If a particular site still doesn’t open, try it in Site check.');
  });
</script>

<Modal title={t('What’s wrong?')} {onclose}>
  <p class="hint">{t('Kova checks the connection step by step, from your network to the server, and says what to change.')}</p>
  {#if checks}
    <p class="verdict">{verdict}</p>
    <CheckList {checks} close={onclose} onfixed={() => setTimeout(run, 1500)} />
  {:else}
    <p class="muted">{t('Checking…')}</p>
  {/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>{t('Close')}</button>
    <button class="btn" class:busy disabled={busy} onclick={run}><Icon name="refresh" />{t('Check again')}</button>
  {/snippet}
</Modal>

<style>
  .hint { margin: 0 0 10px; font-size: 12.5px; color: var(--muted); }
  .verdict { margin: 0 0 6px; font-weight: 600; font-size: 13.5px; }
  .muted { color: var(--muted); }
</style>
