<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, type Rule } from '../lib/api';
  import { t, tn } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { onclose }: { onclose: () => void } = $props();

  // Kova's own processes are routed by Kova already.
  const OWN = new Set(['kova.exe', 'mihomo.exe', 'xray.exe', 'kova', 'mihomo', 'xray']);

  /** Programs with connections open right now, busiest first. */
  let seen = $state<{ name: string; count: number }[] | null>(null);
  void app.run(() => api.getConnections()).then((snap) => {
    const counts = new Map<string, number>();
    for (const c of snap?.connections ?? []) {
      if (c.process && !OWN.has(c.process.toLowerCase())) counts.set(c.process, (counts.get(c.process) ?? 0) + 1);
    }
    seen = [...counts].map(([name, count]) => ({ name, count })).sort((a, b) => b.count - a.count);
  });

  const ruleFor = (name: string) => app.settings?.core.custom_rules.find((r) => r.kind === 'process-name' && r.value.toLowerCase() === name.toLowerCase());

  /** Route `name` one way, above every other rule; replaces an earlier rule for it. */
  async function route(name: string, target: string) {
    const rule: Rule = { kind: 'process-name', value: name, target };
    await app.saveSettings((x) => {
      x.core.custom_rules = [rule, ...x.core.custom_rules.filter((r) => !(r.kind === 'process-name' && r.value.toLowerCase() === name.toLowerCase()))];
    }, target === 'DIRECT' ? t('{name} now goes direct', { name }) : t('{name} now goes through the proxy', { name }));
  }

  async function browse() {
    const path = await open({ multiple: false, filters: [{ name: t('Programs'), extensions: ['exe'] }] });
    if (!path) return;
    const name = String(path).split(/[\\/]/).pop() ?? '';
    if (name && !(seen ?? []).some((s) => s.name.toLowerCase() === name.toLowerCase())) seen = [{ name, count: 0 }, ...(seen ?? [])];
  }
</script>

<Modal title={t('Route an app')} {onclose}>
  <p class="hint">{t('Send a program through the proxy or around it, whatever site it opens. These rules go above all others. In system-proxy mode only programs that use the system proxy are affected; TUN mode covers every program.')}</p>
  <div class="tools"><button class="btn" onclick={browse}><Icon name="file" />{t('Choose a program…')}</button></div>
  {#if seen === null}
    <p class="muted">{t('Loading…')}</p>
  {:else if !seen.length}
    <p class="muted">{app.status.running ? t('No programs have connections right now. Choose one with the button above.') : t('Connect to see which programs are using the network, or choose one with the button above.')}</p>
  {:else}
    <div class="list">
      {#each seen as a (a.name)}
        {@const r = ruleFor(a.name)}
        <div class="row">
          <span class="nm"><span class="t mono">{a.name}</span>
            <span class="d">{r ? (r.target === 'DIRECT' ? t('Goes direct') : t('Goes through the proxy')) : a.count ? tn(a.count, '{n} connection now', '{n} connections now') : t('No rule yet')}</span></span>
          <button class="btn small-btn" aria-pressed={r?.target === 'Proxy'} onclick={() => route(a.name, 'Proxy')}>{t('Proxy')}</button>
          <button class="btn small-btn" aria-pressed={r?.target === 'DIRECT'} onclick={() => route(a.name, 'DIRECT')}>{t('Direct')}</button>
        </div>
      {/each}
    </div>
  {/if}
  {#snippet footer()}
    <button class="btn primary" onclick={onclose}>{t('Done')}</button>
  {/snippet}
</Modal>

<style>
  .hint { margin: 0 0 12px; font-size: 12.5px; color: var(--muted); }
  .tools { margin-bottom: 10px; }
  .list { display: flex; flex-direction: column; max-height: 50vh; overflow: auto; border: 1px solid var(--line); border-radius: 8px; }
  .row { display: flex; align-items: center; gap: 8px; padding: 8px 12px; }
  .row + .row { border-top: 1px solid var(--line); }
  .nm { display: flex; flex-direction: column; flex: 1; min-width: 0; }
  .nm .t { font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nm .d { font-size: 11.5px; color: var(--muted); }
  .small-btn { height: 28px; font-size: 12.5px; }
  .small-btn[aria-pressed='true'] { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
  .muted { color: var(--muted); font-size: 13px; }
</style>
