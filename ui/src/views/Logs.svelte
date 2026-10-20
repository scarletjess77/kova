<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';

  const LEVELS = ['debug', 'info', 'warning', 'error'] as const;
  const LABEL = { debug: 'DBG', info: 'INF', warning: 'WRN', error: 'ERR' } as Record<string, string>;
  let min = $state<(typeof LEVELS)[number]>('info');
  let q = $state('');
  let box: HTMLDivElement;
  let follow = $state(true);

  let shown = $derived(app.logs.filter((l) => LEVELS.indexOf(l.level as never) >= LEVELS.indexOf(min) && (!q || l.message.toLowerCase().includes(q.toLowerCase()))).slice(-800));

  // Track the newest line, not the count: the list is capped, so its length stops changing.
  $effect(() => {
    void shown[shown.length - 1]?.id;
    if (follow) tick().then(() => box && (box.scrollTop = box.scrollHeight));
  });

  const onScroll = () => (follow = box.scrollHeight - box.scrollTop - box.clientHeight < 40);
</script>

<div class="page-head">
  <h1>{t('Logs')}</h1>
  <span class="sub">{app.status.running ? t('Live from the core') : t('Not connected')}</span>
  <div class="tools">
    <label class="search"><Icon name="search" /><input class="input" placeholder={t('Filter messages')} bind:value={q} aria-label={t('Filter logs')} /></label>
    <div class="seg" role="group" aria-label={t('Minimum level')}>
      {#each [['debug', t('Debug')], ['info', t('Info')], ['warning', t('Warning')], ['error', t('Error')]] as [v, l] (v)}
        <button aria-pressed={min === v} onclick={() => (min = v as typeof min)}>{l}</button>
      {/each}
    </div>
    <button class="btn" onclick={() => (app.logsPaused = !app.logsPaused)}><Icon name={app.logsPaused ? 'play' : 'pause'} />{app.logsPaused ? t('Resume') : t('Pause')}</button>
    <button class="btn ghost" onclick={() => app.logs.splice(0)}>{t('Clear')}</button>
  </div>
</div>

{#if min === 'debug' && app.settings?.core.log_level !== 'debug'}
  <p class="hint top">{t('The core is logging at “{level}” level. Set it to Debug in Settings to see debug lines.', { level: app.settings?.core.log_level ?? '' })}</p>
{/if}

<div class="logbox selectable" bind:this={box} onscroll={onScroll}>
  {#each shown as l (l.id)}
    <div class="ll"><span class="ts">{l.time}</span><span class="lv {l.level}">{LABEL[l.level]}</span><span class="src">{l.source}</span><span class="msg">{l.message}</span></div>
  {:else}
    <div class="none">{app.status.running ? t('Waiting for log lines…') : t('Logs appear here while connected.')}</div>
  {/each}
</div>

<style>
  .top { margin: -8px 0 10px; }
  .logbox { background: var(--sunk); border: 1px solid var(--line); border-radius: var(--r); padding: 12px 14px; font: 12px/1.65 var(--f-mono); height: calc(100vh - 190px); overflow: auto; }
  .ll { display: grid; grid-template-columns: auto auto auto minmax(0, 1fr); gap: 12px; white-space: pre-wrap; word-break: break-all; }
  .ts, .src { color: var(--faint); }
  .lv { font-weight: 500; }
  .lv.debug { color: var(--faint); } .lv.info { color: var(--fg); } .lv.warning { color: var(--warn); } .lv.error { color: var(--bad); }
  .none { color: var(--muted); }
</style>
