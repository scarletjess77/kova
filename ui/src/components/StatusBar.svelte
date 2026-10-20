<script lang="ts">
  import { fmtSpeed } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';

  const coreName = (c: string | null) => (c === 'xray' ? 'Xray' : c === 'mihomo' ? 'Mihomo' : '');
  let s = $derived(app.status);
  let st = $derived(app.settings);
  let modeName = $derived(st ? { rule: t('Rule mode'), global: t('Global mode'), direct: t('Direct mode') }[st.core.mode] : '');
</script>

<footer class="status">
  <span><span class="led" class:on={s.running}></span>{s.busy ? t('Working…') : s.running ? t('Running · {core}', { core: coreName(s.core) + (s.sidecar ? ' + Xray' : '') }) : t('Stopped')}</span>
  {#if st}
    <span>{t('Port')} <span class="num">{st.core.mixed_port}</span></span>
    <span>{[s.tun_active && 'TUN', st.system_proxy && t('System proxy')].filter(Boolean).join(' + ') || t('Proxy port only')}</span>
    <span>{modeName}</span>
  {/if}
  <span class="sp"></span>
  <span class="dn">↓ <span class="num">{fmtSpeed(app.down[app.down.length - 1])}</span></span>
  <span class="up">↑ <span class="num">{fmtSpeed(app.up[app.up.length - 1])}</span></span>
</footer>

<style>
  .status { grid-column: 1 / -1; display: flex; align-items: center; gap: 16px; padding: 0 14px; border-top: 1px solid var(--line); font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; }
  .sp { flex: 1; }
  .num { font-size: 11.5px; }
  .led { width: 7px; height: 7px; border-radius: 50%; background: var(--faint); display: inline-block; margin-right: 6px; }
  .led.on { background: var(--ok); box-shadow: 0 0 0 3px var(--ok-soft); }
</style>
