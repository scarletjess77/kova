<script lang="ts">
  import { fmtSpeed } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import type { StatusId } from '../lib/layout';
  import Icon from './Icon.svelte';

  const coreName = (c: string | null) => (c === 'xray' ? 'Xray' : c === 'mihomo' ? 'Mihomo' : '');
  let s = $derived(app.status);
  let st = $derived(app.settings);
  let modeName = $derived(st ? { rule: t('Rule mode'), global: t('Global mode'), direct: t('Direct mode') }[st.core.mode] : '');
  let items = $derived(app.layout.status ?? []);
  let editing = $derived(app.editingLayout);
  const hidden = (id: StatusId) => items.find((x) => x.id === id)?.hidden ?? false;
  /** Shown normally; while arranging, every item shows, hidden ones faded, each with its eye. */
  const on = (id: StatusId) => editing || !hidden(id);
  const toggle = (id: StatusId) => app.saveLayout({ ...app.layout, status: items.map((x) => (x.id === id ? { id, ...(x.hidden ? {} : { hidden: true }) } : x)) });
  let NAME = $derived<Record<StatusId, string>>({ state: t('Connection state'), port: t('Port'), capture: t('Capture'), mode: t('Routing mode'), down: t('Download'), up: t('Upload') });
</script>

{#snippet eye(id: StatusId)}
  {#if editing}<button class="eye" onclick={() => toggle(id)} aria-pressed={!hidden(id)} aria-label={hidden(id) ? t('Show {name}', { name: NAME[id] }) : t('Hide {name}', { name: NAME[id] })}><Icon name={hidden(id) ? 'eye-off' : 'eye'} /></button>{/if}
{/snippet}

<footer class="status" class:editing>
  {#if on('state')}<span class:off={hidden('state')}><span class="led" class:on={s.running}></span>{s.busy ? t('Working…') : s.running ? t('Running · {core}', { core: coreName(s.core) + (s.bridged.length ? ' + Xray' : '') }) : t('Stopped')}{@render eye('state')}</span>{/if}
  {#if st}
    {#if on('port')}<span class:off={hidden('port')}>{t('Port')} <span class="num">{st.core.mixed_port}</span>{@render eye('port')}</span>{/if}
    {#if on('capture') && (s.running || editing)}<span class:off={hidden('capture')}>{[s.tun_active && 'TUN', st.system_proxy && t('System proxy')].filter(Boolean).join(' + ') || t('Proxy port only')}{@render eye('capture')}</span>{/if}
    {#if on('mode')}<span class:off={hidden('mode')}>{modeName}{@render eye('mode')}</span>{/if}
  {/if}
  <span class="sp"></span>
  {#if on('down')}<span class="dn" class:off={hidden('down')}>↓ <span class="num">{fmtSpeed(app.down[app.down.length - 1])}</span>{@render eye('down')}</span>{/if}
  {#if on('up')}<span class="up" class:off={hidden('up')}>↑ <span class="num">{fmtSpeed(app.up[app.up.length - 1])}</span>{@render eye('up')}</span>{/if}
</footer>

<style>
  .status { grid-column: 1 / -1; display: flex; align-items: center; gap: 16px; padding: 0 14px; border-top: 1px solid var(--line); font-size: 12px; color: var(--muted); white-space: nowrap; overflow: hidden; }
  .status > span { display: inline-flex; align-items: center; gap: 4px; }
  .status.editing > span:not(.sp) { padding: 0 2px 0 6px; border: 1px dashed var(--line-2); border-radius: 6px; }
  .off { opacity: .45; }
  .sp { flex: 1; }
  .num { font-size: 12px; }
  .led { width: 7px; height: 7px; border-radius: 50%; background: var(--faint); display: inline-block; margin-inline-end: 6px; }
  .led.on { background: var(--ok); box-shadow: 0 0 0 3px var(--ok-soft); }
  .eye { display: inline-grid; place-items: center; padding: 2px; border: 0; background: none; color: var(--faint); }
  .eye :global(.i) { width: 13px; height: 13px; }
</style>
