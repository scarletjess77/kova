<script lang="ts">
  import { onDestroy } from 'svelte';
  import { countryCode, displayName } from '../lib/format';
  import { app } from '../lib/state.svelte';
  import Icon from '../components/Icon.svelte';
  import Latency from '../components/Latency.svelte';

  const KIND: Record<string, string> = { Selector: 'select', URLTest: 'url-test', Fallback: 'fallback', LoadBalance: 'load-balance', Relay: 'relay' };
  const NOTE: Record<string, string> = {
    Selector: 'You choose which member carries traffic.',
    URLTest: 'Re-tests every few minutes and moves to the fastest member.',
    Fallback: 'Uses the first member that passes health checks.',
    LoadBalance: 'Spreads connections across members.',
  };

  let groupName = $state<string | null>(null);
  let q = $state('');
  let sort = $state<'latency' | 'name' | 'profile'>('profile');

  let groups = $derived(app.proxies.groups.filter((g) => g.name !== 'GLOBAL' || app.settings?.core.mode === 'global'));
  let group = $derived(groups.find((g) => g.name === groupName) ?? groups[0]);
  let isGroup = (n: string) => app.proxies.groups.some((g) => g.name === n);

  let members = $derived.by(() => {
    if (!group) return [];
    const needle = q.trim().toLowerCase();
    let list = group.all.filter((n) => {
      if (!needle) return true;
      const info = app.nodes[n];
      return [n, info?.protocol, info?.transport, info?.server, countryCode(n)].join(' ').toLowerCase().includes(needle);
    });
    if (sort === 'latency') list = [...list].sort((a, b) => Number(isGroup(b)) - Number(isGroup(a)) || (app.delayOf(a) ?? 1e9) - (app.delayOf(b) ?? 1e9));
    if (sort === 'name') list = [...list].sort((a, b) => displayName(a).localeCompare(displayName(b)));
    return list;
  });

  function pick(name: string) {
    if (!group) return;
    if (group.type !== 'Selector') {
      app.notify(`${group.name} chooses automatically. Pick a node in a select group to pin one.`);
      return;
    }
    void app.select(group.name, name);
  }

  function testGroup() {
    if (!group) return;
    const names = group.all.filter((n) => !isGroup(n) && !['DIRECT', 'REJECT'].includes(n));
    void app.testDelays(names.length ? names : undefined);
  }

  // Keep group selections fresh while the core moves url-test groups around.
  const poll = setInterval(() => { if (app.status.running) void app.refreshProxies(); }, 5000);
  onDestroy(() => clearInterval(poll));
  void app.refreshProxies();
</script>

<div class="page-head">
  <h1>Proxies</h1>
  <span class="sub">{groups.length} groups · {Object.keys(app.nodes).length} nodes</span>
  <div class="tools">
    <label class="search"><Icon name="search" /><input class="input" placeholder="Search nodes, countries, protocols" bind:value={q} aria-label="Search nodes" /></label>
    <select class="input sort" bind:value={sort} aria-label="Sort">
      <option value="profile">Profile order</option>
      <option value="latency">Sort by latency</option>
      <option value="name">Sort by name</option>
    </select>
    <button class="btn primary" onclick={testGroup} disabled={!group || app.testing.size > 0}><Icon name="bolt" />Test latency</button>
  </div>
</div>

{#if !groups.length}
  <div class="empty">
    <b>No nodes yet</b>
    <span>Add a subscription or share links in Profiles.</span>
    <button class="btn primary" onclick={() => { app.view = 'profiles'; app.importOpen = true; }}><Icon name="plus" />Add profile</button>
  </div>
{:else}
  <div class="px">
    <div class="groups">
      {#each groups as g (g.name)}
        {@const end = app.resolve(g.name)}
        <button class="grp" onclick={() => (groupName = g.name)} aria-current={g.name === group?.name}>
          <span class="top"><b>{g.name}</b><span class="chip mono">{KIND[g.type] ?? g.type.toLowerCase()}</span></span>
          <span class="now">{g.now && g.now !== end ? `${displayName(g.now)} › ` : ''}{displayName(end)}{app.delays[end] != null ? ` · ${app.delays[end]} ms` : ''}</span>
        </button>
      {/each}
    </div>
    <div>
      {#if group}
        <p class="grp-note">{NOTE[group.type] ?? ''}{#if group.type !== 'Selector'} <span class="chip">Automatic</span>{/if}</p>
        {#if members.length}
          <div class="nodes">
            {#each members as name (name)}
              {@const info = app.nodes[name]}
              {@const sub = isGroup(name)}
              <button class="node" class:is-group={sub} aria-pressed={group.now === name} onclick={() => pick(name)}>
                <span class="r1">
                  <span class="cc">{#if sub}<Icon name="bolt" />{:else}{countryCode(name) ?? '··'}{/if}</span>
                  <b title={name}>{displayName(name)}</b>
                  <Latency delay={app.delayOf(name)} testing={app.testing.has(app.resolve(name))} />
                </span>
                {#if sub}
                  <span class="r2"><span class="chip mono">group</span><span class="tr">now {displayName(app.resolve(name))}</span></span>
                {:else if info}
                  <span class="r2"><span class="chip">{info.protocol}</span><span class="tr">{info.transport}</span></span>
                  <span class="r3"><span class="dot" class:y={info.mihomo}></span>Mihomo <span class="dot" class:y={info.xray}></span>Xray
                    {#if app.status.bridged.includes(name)}<span class="via">· via Xray bridge</span>{/if}</span>
                {:else}
                  <span class="r2"><span class="tr">{app.proxies.proxies[name]?.type ?? ''}</span></span>
                {/if}
              </button>
            {/each}
          </div>
        {:else}
          <p class="hint">No nodes match “{q}”.</p>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .sort { width: auto; }
  .px { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: 16px; align-items: start; }
  @container (max-width: 700px) { .px { grid-template-columns: minmax(0, 1fr); } }
  .groups { display: flex; flex-direction: column; gap: 6px; position: sticky; top: 0; }
  .grp { display: flex; flex-direction: column; gap: 3px; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); text-align: left; min-width: 0; }
  .grp:hover { border-color: var(--line-2); }
  .grp[aria-current='true'] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .top b { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .top .chip { margin-left: auto; height: 19px; font-size: 10.5px; }
  .now { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grp-note { font-size: 12.5px; color: var(--muted); margin: 0 0 12px; display: flex; gap: 8px; align-items: center; min-height: 22px; }
  .nodes { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 10px; }
  .node { display: flex; flex-direction: column; gap: 9px; padding: 12px; border: 1px solid var(--line); border-radius: var(--r); background: var(--panel); text-align: left; min-width: 0; }
  .node:hover { border-color: var(--line-2); }
  .node[aria-pressed='true'] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .r1 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .r1 b { font-weight: 600; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r2 { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; min-width: 0; }
  .tr { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r3 { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--faint); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--line-2); }
  .dot.y { background: var(--fg); }
  .via { color: var(--muted); }
  .is-group .cc { background: var(--accent); color: var(--accent-fg); }
</style>
