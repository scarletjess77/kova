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
  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

  /** Programs with connections open right now, with how many. */
  let busy = $state<Map<string, number> | null>(null);
  /** Every program of this user running now. */
  let running = $state<string[]>([]);
  /** Programs picked with the file dialog this time. */
  let picked = $state<string[]>([]);
  let q = $state('');

  void app.run(() => api.getConnections()).then((snap) => {
    const counts = new Map<string, number>();
    for (const c of snap?.connections ?? []) {
      if (c.process && !OWN.has(c.process.toLowerCase())) counts.set(c.process, (counts.get(c.process) ?? 0) + 1);
    }
    busy = counts;
  });
  void api.runningPrograms().then((names) => (running = names), () => {});

  let rules = $derived((app.settings?.core.custom_rules ?? []).filter((r) => r.kind === 'process-name'));
  const ruleFor = (name: string) => rules.find((r) => same(r.value, name));
  const isBusy = (name: string) => [...(busy?.keys() ?? [])].some((b) => same(b, name));
  const matches = (name: string) => !q.trim() || name.toLowerCase().includes(q.trim().toLowerCase());

  // Three lists, each program in one: routed already, using the network, the rest.
  let routed = $derived(rules.map((r) => r.value).filter(matches));
  let networked = $derived([...(busy ?? new Map<string, number>()).entries()].sort((a, b) => b[1] - a[1]).map(([n]) => n).filter((n) => !ruleFor(n) && matches(n)));
  let others = $derived([...picked, ...running].filter((n, i, all) => all.findIndex((m) => same(m, n)) === i && !ruleFor(n) && !isBusy(n) && matches(n)));

  /** Route `name` one way, above every other rule; replaces an earlier rule for it. */
  async function route(name: string, target: string) {
    const rule: Rule = { kind: 'process-name', value: name, target };
    const done = target === 'DIRECT' ? t('{name} now goes direct', { name }) : target === 'REJECT' ? t('{name} is now blocked', { name }) : t('{name} now goes through the proxy', { name });
    await app.saveSettings((x) => {
      x.core.custom_rules = [rule, ...x.core.custom_rules.filter((r) => !(r.kind === 'process-name' && same(r.value, name)))];
    }, done);
  }

  async function unroute(name: string) {
    await app.saveSettings((x) => {
      x.core.custom_rules = x.core.custom_rules.filter((r) => !(r.kind === 'process-name' && same(r.value, name)));
    }, t('Rule for {name} removed', { name }));
  }

  async function browse() {
    // Rules match the program's file name: name.exe on Windows, the app's name on a Mac.
    const filters = app.platform === 'windows' ? [{ name: t('Programs'), extensions: ['exe'] }] : undefined;
    const path = await open({ multiple: false, filters });
    if (!path) return;
    const name = (String(path).replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? '').replace(/\.app$/i, '');
    if (name && !picked.some((p) => same(p, name))) picked = [name, ...picked];
  }

  const describe = (name: string) => {
    const r = ruleFor(name);
    if (r) return r.target === 'DIRECT' ? t('Goes direct') : r.target === 'REJECT' ? t('Blocked') : t('Goes through the proxy');
    const n = [...(busy?.entries() ?? [])].find(([b]) => same(b, name))?.[1] ?? 0;
    return n ? tn(n, '{n} connection now', '{n} connections now') : t('No rule yet');
  };
</script>

{#snippet row(name: string)}
  {@const r = ruleFor(name)}
  <div class="row">
    <span class="nm"><span class="t mono">{name}</span><span class="d">{describe(name)}</span></span>
    <button class="btn small-btn" aria-pressed={r?.target === 'Proxy'} onclick={() => route(name, 'Proxy')}>{t('Proxy')}</button>
    <button class="btn small-btn" aria-pressed={r?.target === 'DIRECT'} onclick={() => route(name, 'DIRECT')}>{t('Direct')}</button>
    <button class="btn small-btn" aria-pressed={r?.target === 'REJECT'} onclick={() => route(name, 'REJECT')}>{t('Block')}</button>
    {#if r}<button class="btn ghost icon-btn" onclick={() => unroute(name)} aria-label={t('Remove the rule for {name}', { name })} title={t('Remove the rule for {name}', { name })}><Icon name="x" /></button>{/if}
  </div>
{/snippet}

<Modal title={t('Route an app')} {onclose}>
  <p class="hint">{t('Send a program through the proxy or around it, whatever site it opens. These rules go above all others. In system-proxy mode only programs that use the system proxy are affected; TUN mode covers every program.')}</p>
  <div class="tools">
    <input class="input" type="search" bind:value={q} placeholder={t('Search programs')} aria-label={t('Search programs')} />
    <button class="btn" onclick={browse}><Icon name="file" />{t('Choose a program…')}</button>
  </div>
  {#if busy === null}
    <p class="muted">{t('Loading…')}</p>
  {:else if !routed.length && !networked.length && !others.length}
    <p class="muted">{t('No programs match')}</p>
  {:else}
    <div class="list">
      {#if routed.length}
        <div class="sec">{t('Routed apps')}</div>
        {#each routed as name (name)}{@render row(name)}{/each}
      {/if}
      {#if networked.length}
        <div class="sec">{t('Using the network now')}</div>
        {#each networked as name (name)}{@render row(name)}{/each}
      {/if}
      {#if others.length}
        <div class="sec">{t('Other running programs')}</div>
        {#each others as name (name)}{@render row(name)}{/each}
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <button class="btn primary" onclick={onclose}>{t('Done')}</button>
  {/snippet}
</Modal>

<style>
  .hint { margin: 0 0 12px; font-size: 12.5px; color: var(--muted); }
  .tools { display: flex; gap: 8px; margin-bottom: 10px; }
  .tools .input { flex: 1; min-width: 0; }
  .list { display: flex; flex-direction: column; max-height: 50vh; overflow: auto; border: 1px solid var(--line); border-radius: 8px; }
  .sec { position: sticky; top: 0; z-index: 1; padding: 6px 12px; font-size: 11.5px; font-weight: 600; color: var(--muted); background: var(--sunk); border-bottom: 1px solid var(--line); }
  .row { display: flex; align-items: center; gap: 8px; padding: 8px 12px; }
  .row + .row { border-top: 1px solid var(--line); }
  .nm { display: flex; flex-direction: column; flex: 1; min-width: 0; }
  .nm .t { font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .nm .d { font-size: 11.5px; color: var(--muted); }
  .small-btn { height: 28px; font-size: 12.5px; }
  .small-btn[aria-pressed='true'] { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
  .icon-btn { height: 28px; width: 28px; padding: 0; justify-content: center; }
  .muted { color: var(--muted); font-size: 13px; }
</style>
