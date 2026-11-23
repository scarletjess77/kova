<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type Learned, type Preset, type Rule, type RuleKind, type Suggestion, type TorrentRoute } from '../lib/api';
  import { countryCode, fmtAgo } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import AppRules from '../components/AppRules.svelte';
  import Icon from '../components/Icon.svelte';

  let PRESETS = $derived<{ id: Preset; title: string; desc: string; needs?: 'runetfreedom' }[]>([
    { id: 'proxy-blocked-ru', title: t('Only blocked sites through the proxy'), desc: t('Sites blocked in Russia use the proxy; everything else goes direct. Uses the Russia routing data.'), needs: 'runetfreedom' },
    { id: 'direct-ru', title: t('Direct for Russian sites'), desc: t('.ru, .рф, .su, Russian services and IP ranges skip the proxy.') },
    { id: 'block-ads', title: t('Block ads and trackers'), desc: t('Rejects the category-ads-all list.') },
    { id: 'direct-cn', title: t('Direct for Chinese sites'), desc: t('geosite:cn and Chinese IP ranges go direct.') },
    { id: 'direct-ir', title: t('Direct for Iranian sites'), desc: t('.ir domains and Iranian IP ranges go direct.') },
  ]);
  let KINDS = $derived<[RuleKind, string, string][]>([
    ['domain-suffix', t('Domain and subdomains'), 'example.org'],
    ['domain', t('Exact domain'), 'api.github.com'],
    ['domain-keyword', t('Domain contains'), 'google'],
    ['geosite', t('Geosite list'), 'youtube'],
    ['geoip', t('Country (GeoIP)'), 'DE'],
    ['ip-cidr', t('IP range'), '203.0.113.0/24'],
    ['process-name', t('App (process name)'), 'steam.exe'],
  ]);
  let KIND_LABEL = $derived(Object.fromEntries(KINDS.map(([k, l]) => [k, l])));

  let kind = $state<RuleKind>('domain-suffix');
  let value = $state('');
  let target = $state('Proxy');
  let routingApps = $state(false);

  // Edits here wait for Apply (each one would restart the connection), so the page shows
  // the saved settings with those edits on top.
  let s = $derived(app.ruleSettings);
  let rules = $derived(s?.core.custom_rules ?? []);
  // Countries the active profile has servers in: a rule can send a site through the
  // fastest of them (`country:US`; the engine makes the group).
  let countries = $derived([...new Set([...Object.keys(app.proxies.proxies), ...Object.keys(app.nodes)].map(countryCode).filter((c): c is string => !!c))].sort());
  let targets = $derived(['Proxy', 'DIRECT', 'REJECT', ...countries.map((c) => `country:${c}`), ...app.proxies.groups.map((g) => g.name).filter((g) => !['Proxy', 'GLOBAL'].includes(g) && !g.endsWith(' · fastest'))]);
  function targetLabel(x: string): string {
    if (x === 'Proxy') return t('Proxy (main group)');
    const c = x.startsWith('country:') ? x.slice(8) : null;
    return c ? t('Fastest in {country}', { country: c }) : x;
  }

  // AI services block Russia and some other countries: one rule sends them all abroad.
  const AI_LIST = 'category-ai-!cn';
  let aiCountry = $state('');
  let hasAiRule = $derived(rules.some((r) => r.kind === 'geosite' && r.value === AI_LIST));
  $effect(() => {
    if (!countries.includes(aiCountry)) aiCountry = countries.includes('US') ? 'US' : (countries[0] ?? '');
  });
  function addAiRule() {
    const c = aiCountry;
    if (!c) return;
    const rule: Rule = { kind: 'geosite', value: AI_LIST, target: `country:${c}` };
    app.stageRules((x) => (x.core.custom_rules = [rule, ...x.core.custom_rules]));
  }
  // Each row's explanation is folded away until its arrow is pressed; which rows are open is
  // kept on this device, by row id. Warnings never fold.
  const OPEN_KEY = 'kova.rulesOpen';
  const OPEN_IDS = () => [...PRESETS.map((p) => p.id), 'torrent', 'apply', 'smart', 'smart-profile'];
  let openRows = $state<Record<string, boolean>>((() => {
    // The old "show all explanations" switch is gone: whoever had it on starts with every row open.
    try {
      const old = localStorage.getItem('kova.rulesHelp');
      if (old !== null) {
        if (old === '1' && localStorage.getItem(OPEN_KEY) === null) {
          localStorage.setItem(OPEN_KEY, JSON.stringify(Object.fromEntries(OPEN_IDS().map((id) => [id, true]))));
        }
        localStorage.removeItem('kova.rulesHelp');
      }
    } catch { /* the old choice is just not carried over */ }
    try {
      const v = JSON.parse(localStorage.getItem(OPEN_KEY) ?? '{}');
      return v && typeof v === 'object' && !Array.isArray(v) ? v : {};
    } catch { return {}; }
  })());
  const isOpen = (id: string) => !!openRows[id];
  function fold(id: string) {
    openRows[id] = !openRows[id];
    try { localStorage.setItem(OPEN_KEY, JSON.stringify(openRows)); } catch { /* closed again next time */ }
  }
  let isClash = $derived(app.activeProfile?.format === 'clash');

  function togglePreset(p: Preset) {
    app.stageRules((x) => {
      x.core.presets = x.core.presets.includes(p) ? x.core.presets.filter((y) => y !== p) : [...x.core.presets, p];
    });
  }

  // null leaves torrent clients to the other rules.
  let TORRENT = $derived<[TorrentRoute | null, string][]>([[null, t('Not set')], ['proxy', t('Through the proxy')], ['direct', t('Direct')], ['block', t('Block')]]);
  const setTorrent = (v: TorrentRoute | null) => app.stageRules((x) => (x.core.torrent = v));

  function add() {
    const v = value.trim();
    if (!v) return;
    const rule: Rule = { kind, value: v, target };
    // On top: the newest rule wins over older ones that match the same sites.
    app.stageRules((x) => (x.core.custom_rules = [rule, ...x.core.custom_rules]));
    value = '';
  }

  function move(i: number, d: -1 | 1) {
    app.stageRules((x) => {
      const r = [...x.core.custom_rules];
      const j = i + d;
      if (j < 0 || j >= r.length) return;
      [r[i], r[j]] = [r[j], r[i]];
      x.core.custom_rules = r;
    });
  }

  // Drag to reorder. The rows stay where they are in the page while dragging: the
  // dragged one follows the pointer and the others slide aside with a transform, and
  // the order only changes on release. (Moving the row element itself would make the
  // browser drop the pointer capture, and the release would never arrive.) Pointer
  // events rather than HTML drag-and-drop, which the desktop webview keeps for files.
  let tbody = $state<HTMLTableSectionElement>();
  let drag = $state<{ from: number; to: number; dy: number; height: number } | null>(null);
  // Positions are measured from the top of the table body, which itself never moves
  // relative to its rows, so scrolling the page mid-drag doesn't throw the target off.
  let startY = 0;
  let lastClientY = 0;
  let mids: number[] = [];
  const tableTop = () => tbody?.getBoundingClientRect().top ?? 0;

  function grab(e: PointerEvent, i: number) {
    if (e.button !== 0 || !tbody) return;
    e.preventDefault();
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    const top = tableTop();
    const rects = [...tbody.rows].map((r) => r.getBoundingClientRect());
    mids = rects.map((b) => b.top - top + b.height / 2);
    lastClientY = e.clientY;
    startY = e.clientY - top;
    drag = { from: i, to: i, dy: 0, height: rects[i].height };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', cancelDrag);
    window.addEventListener('blur', cancelDrag);
    window.addEventListener('keydown', onEscape);
    window.addEventListener('scroll', follow, true);
  }

  function onMove(e: PointerEvent) {
    if (!drag) return;
    // Released somewhere we couldn't hear about (outside the window): finish now.
    if ((e.buttons & 1) === 0) return onUp();
    lastClientY = e.clientY;
    follow();
  }

  /** Place the dragged row under the pointer; also called when the page scrolls. */
  function follow() {
    if (!drag) return;
    const y = lastClientY - tableTop();
    // The new position is the number of other rows whose middle is above the pointer.
    const to = mids.filter((m, j) => j !== drag!.from && m < y).length;
    drag = { ...drag, to, dy: y - startY };
  }

  function onUp() {
    const d = drag;
    endDrag();
    if (d && d.to !== d.from) {
      const next = [...rules];
      const [moved] = next.splice(d.from, 1);
      next.splice(d.to, 0, moved);
      app.stageRules((x) => (x.core.custom_rules = next));
    }
  }

  function onEscape(e: KeyboardEvent) {
    if (e.key === 'Escape') cancelDrag();
  }

  function cancelDrag() {
    endDrag();
  }

  function endDrag() {
    drag = null;
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', cancelDrag);
    window.removeEventListener('blur', cancelDrag);
    window.removeEventListener('keydown', onEscape);
    window.removeEventListener('scroll', follow, true);
  }

  // Leaving the page mid-drag must not leave window listeners behind.
  onDestroy(endDrag);

  /** How far row `j` is shifted while a drag is in progress. */
  function shift(j: number): number {
    if (!drag) return 0;
    const { from, to, dy, height } = drag;
    if (j === from) return dy;
    if (from < to && j > from && j <= to) return -height;
    if (to < from && j >= to && j < from) return height;
    return 0;
  }

  function gripKey(e: KeyboardEvent, k: number) {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      move(k, e.key === 'ArrowUp' ? -1 : 1);
    }
  }

  // Smart routing
  let learned = $state<Learned[]>([]);
  let suggestions = $state<Suggestion[]>([]);
  const loadLearned = async () => {
    learned = (await app.run(() => api.learnedSites())) ?? learned;
    suggestions = (await app.run(() => api.smartSuggestions())) ?? suggestions;
    app.suggestionCount = suggestions.length;
  };

  // How Mihomo names a rule in its logs, to find which of the user's rules a
  // suggestion is about.
  const MIHOMO_NAME: Record<RuleKind, string> = {
    domain: 'Domain', 'domain-suffix': 'DomainSuffix', 'domain-keyword': 'DomainKeyword', geosite: 'GeoSite',
    geoip: 'GeoIP', 'ip-cidr': 'IPCIDR', 'process-name': 'ProcessName', match: 'Match',
  };
  const ruleKey = (r: Rule) => (r.value ? `${MIHOMO_NAME[r.kind]}(${r.value})` : MIHOMO_NAME[r.kind]);

  /** Which rule sent the site the failing way, in words. */
  function culprit(sg: Suggestion): string {
    if (sg.origin === 'user') {
      const i = rules.findIndex((r) => ruleKey(r) === sg.rule);
      if (i >= 0) return t('your rule #{n} ({kind} {value})', { n: i + 1, kind: KIND_LABEL[rules[i].kind] ?? rules[i].kind, value: rules[i].value });
      return t('one of your rules ({rule})', { rule: sg.rule });
    }
    if (sg.origin === 'site-check') return t('Kova’s rules');
    return sg.origin === 'preset' ? t('a routing preset ({rule})', { rule: sg.rule }) : t('the subscription’s rule {rule}', { rule: sg.rule });
  }

  async function acceptSuggestion(sg: Suggestion) {
    // The engine changes the rules itself here: what waits for Apply goes in first, so
    // the two don't overwrite each other.
    await app.applyRules(false);
    const saved = await app.run(() => api.acceptSuggestion(sg.domain), t('Exception added: {domain} now goes {way}', { domain: sg.domain, way: sg.route === 'proxy' ? t('through the proxy') : t('direct') }));
    if (saved) app.settings = saved;
    await loadLearned();
  }

  async function dismissSuggestion(sg: Suggestion) {
    await app.run(() => api.dismissSuggestion(sg.domain));
    await loadLearned();
  }
  $effect(() => { void app.learnedVersion; void loadLearned(); });
  let REASON = $derived<Record<string, string>>({
    timeout: t('no answer'), reset: t('connection reset'), tls: t('secure connection broken'), dns: t('name not resolved'),
    'connect-failed': t('could not connect'), refused: t('access refused'),
  });
  async function forget(domain: string) {
    await app.run(() => api.forgetLearned(domain), t('{domain} forgotten', { domain }));
    await loadLearned();
  }
  async function keep(domain: string) {
    await app.applyRules(false);
    const saved = await app.run(() => api.keepLearned(domain), t('{domain} moved to your rules', { domain }));
    if (saved) app.settings = saved;
    await loadLearned();
  }
  async function clearAll() {
    await app.run(() => api.clearLearned(), t('Learned sites cleared'));
    await loadLearned();
  }

  const remove = (i: number) => app.stageRules((x) => (x.core.custom_rules = x.core.custom_rules.filter((_, k) => k !== i)));
  const polClass = (t: string) => (t === 'DIRECT' ? 'direct' : t === 'REJECT' ? 'reject' : 'proxy');
</script>

<div class="page-head">
  <h1>{t('Rules')}</h1><span class="sub">{t('The first matching rule decides where traffic goes')}</span>
</div>

{#if s}
  {#if app.rulesDraft}
    <div class="pending" role="status">
      <span>{app.status.running ? t('Changes are not applied yet. Apply them all at once; the connection restarts once.') : t('Changes are not applied yet.')}</span>
      <div class="ops">
        <button class="btn ghost" disabled={app.applyingRules} onclick={() => app.discardRules()}>{t('Discard')}</button>
        <button class="btn primary" class:busy={app.applyingRules} disabled={app.applyingRules} onclick={() => app.applyRules()}><Icon name="check" />{t('Apply')}</button>
      </div>
    </div>
  {/if}
  <section class="presets">
    <div class="card-h"><h2>{t('Routing presets')}</h2></div>
    <div class="pgrid">
      {#each PRESETS as p (p.id)}
        {@const unavailable = p.needs === 'runetfreedom' && s.core.geodata.kind === 'default' && !s.core.presets.includes(p.id)}
        <div class="tile card">
          <div class="line">
            <button class="fold" title={p.title} aria-expanded={isOpen(p.id)} aria-controls={isOpen(p.id) ? `rr-${p.id}` : undefined} onclick={() => fold(p.id)}><Icon name="chevron" /><span class="t">{p.title}</span></button>
            <button class="switch" role="switch" aria-label={p.title} aria-checked={s.core.presets.includes(p.id)} disabled={unavailable} title={unavailable ? t('Switch routing data to Russia in Settings') : undefined} onclick={() => togglePreset(p.id)}></button>
          </div>
          <!-- The way to turn an unavailable preset on is inside its description and in the switch's tooltip, not a line of its own. -->
          {#if isOpen(p.id)}<div class="d" id="rr-{p.id}">{p.desc}{#if unavailable} <button class="link" onclick={() => app.go('settings')}>{t('Switch routing data to Russia in Settings')}</button>{/if}</div>{/if}
        </div>
      {/each}
      {#if !app.mobile}
        <div class="tile card tor-tile">
          <div class="line tor">
            <button class="fold" title={t('Torrent clients')} aria-expanded={isOpen('torrent')} aria-controls={isOpen('torrent') ? 'rr-torrent' : undefined} onclick={() => fold('torrent')}><Icon name="chevron" /><span class="t">{t('Torrent clients')}</span></button>
            <div class="seg" role="group" aria-label={t('Torrent clients')}>
              {#each TORRENT as [v, l] (v ?? 'none')}
                <button aria-pressed={(s.core.torrent ?? null) === v} onclick={() => setTorrent(v)}>{l}</button>
              {/each}
            </div>
          </div>
          {#if isOpen('torrent')}<div class="d" id="rr-torrent">{t('Route torrent programs (qBittorrent, Transmission, Deluge…) on their own. Only programs whose traffic goes through Kova are affected.')}</div>{/if}
          {#if s.core.torrent && isClash && !s.core.apply_rules_to_clash_profiles}
            <div class="d warn-text">{t('The torrent choice is ignored for the active Clash profile. Turn on “Apply to imported Clash profiles” below.')}</div>
          {/if}
          {#if s.core.torrent === 'proxy'}
            <div class="d warn-text">{t('Many providers forbid torrents on their servers. This can get your account blocked.')}</div>
          {/if}
        </div>
      {/if}
    </div>
  </section>

  <section class="card">
    <div class="card-h"><h2>{t('Your rules')}</h2><span class="aside">{t('checked before presets')}</span>
      {#if !app.mobile}<button class="btn ghost" onclick={() => (routingApps = true)}><Icon name="monitor" />{t('Route an app…')}</button>{/if}</div>
    <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <select class="input" bind:value={kind} aria-label={t('Match type')}>
        {#each KINDS as [k, l] (k)}<option value={k}>{l}</option>{/each}
      </select>
      <input class="input mono" bind:value={value} placeholder={KINDS.find((k) => k[0] === kind)?.[2]} aria-label={t('Value')} />
      <select class="input" bind:value={target} aria-label={t('Send to')}>
        {#each targets as x (x)}<option value={x}>{targetLabel(x)}</option>{/each}
      </select>
      <button class="btn primary" type="submit" disabled={!value.trim()}><Icon name="plus" />{t('Add rule')}</button>
    </form>
    {#if !hasAiRule && countries.length}
      <div class="ai">
        <span>{t('ChatGPT, Claude, Gemini and other AI services through')}</span>
        <select class="input small" bind:value={aiCountry} aria-label={t('Country for AI services')}>
          {#each countries as c (c)}<option value={c}>{c}</option>{/each}
        </select>
        <button class="btn" onclick={addAiRule}><Icon name="plus" />{t('Add rule')}</button>
      </div>
    {/if}
    {#if rules.length}
      <div class="twrap list">
        <table>
          <thead><tr><th></th><th>#</th><th>{t('Match')}</th><th>{t('Value')}</th><th>{t('Send to')}</th><th></th></tr></thead>
          <tbody bind:this={tbody}>
            {#each rules as r, i (i)}
              <tr class:dragging={drag?.from === i} class:moving={drag !== null && drag.from !== i} style:transform={drag ? `translateY(${shift(i)}px)` : null}>
                <td class="grip-cell">
                  <button class="grip" aria-label={t('Move rule {n}: drag, or use the arrow keys', { n: i + 1 })}
                    onpointerdown={(e) => grab(e, i)} onkeydown={(e) => gripKey(e, i)}><Icon name="grip" /></button>
                </td>
                <td class="num muted">{i + 1}</td>
                <td><span class="rt">{KIND_LABEL[r.kind] ?? r.kind}</span></td>
                <td class="mono small selectable">{r.value}</td>
                <td><span class="pol {polClass(r.target)}">{targetLabel(r.target)}</span></td>
                <td class="r"><button class="x" onclick={() => remove(i)} aria-label={t('Delete rule {n}', { n: i + 1 })}><Icon name="x" /></button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if rules.length > 1}<p class="hint">{t('Drag')} <Icon name="grip" class="inline-grip" /> {t('to change the order, or focus it and press ↑ or ↓. Rules higher up win.')}</p>{/if}
    {:else}
      <p class="hint">{t('No rules of your own yet. Add one to send a site or app through the proxy, direct, or block it.')}</p>
    {/if}
    <div class="prow apply">
      <div class="line">
        <button class="fold" aria-expanded={isOpen('apply')} aria-controls={isOpen('apply') ? 'rr-apply' : undefined} onclick={() => fold('apply')}><Icon name="chevron" /><span class="t">{t('Apply to imported Clash profiles')}</span></button>
        <button class="switch" role="switch" aria-label={t('Apply to imported Clash profiles')} aria-checked={s.core.apply_rules_to_clash_profiles}
          onclick={() => app.stageRules((x) => (x.core.apply_rules_to_clash_profiles = !x.core.apply_rules_to_clash_profiles))}></button>
      </div>
      {#if isOpen('apply')}<div class="d" id="rr-apply">{isClash ? t('Put your rules and presets in front of a Clash profile’s own rules (the active profile is one).') : t('Put your rules and presets in front of a Clash profile’s own rules.')}</div>{/if}
    </div>
  </section>

  <section class="card smart">
    <div class="card-h">
      <button class="fold" aria-expanded={isOpen('smart')} aria-controls={isOpen('smart') ? 'rr-smart' : undefined} onclick={() => fold('smart')}><Icon name="chevron" /><h2>{t('Smart routing')}</h2></button>
      <span class="chip">beta</span>
      <button class="switch aside-sw" role="switch" aria-label={t('Smart routing')} aria-checked={s.core.smart_routing}
        onclick={() => app.stageRules((x) => (x.core.smart_routing = !x.core.smart_routing))}></button></div>
    {#if isOpen('smart')}<p class="hint" id="rr-smart">{t('Kova notices sites that fail on their usual path, checks them directly and through the proxy, and sends each one the way that works. With TUN it also learns apps that connect by IP address. Learned sites are kept for 30 days.')}</p>{/if}
    {#if s.core.smart_routing && s.core.mode !== 'rule'}
      <p class="hint warn-text">{t('Learning is paused: it works in Rule mode only.')}</p>
    {:else if s.core.smart_routing && s.core.tun.enabled && !s.core.sniff && !s.core.dns.fake_ip}
      <p class="hint warn-text">{t('In TUN mode Kova needs site names: turn on Sniff domains or Fake-IP in Settings, otherwise it can only learn IP addresses.')}</p>
    {:else if s.core.smart_routing && isClash && !s.core.apply_rules_to_clash_profiles}
      <p class="hint warn-text">{t('Turn on “Apply to imported Clash profiles” above: the active profile is a Clash profile.')}</p>
    {/if}
    {#if s.core.smart_routing && isClash}
      <div class="prow">
        <div class="line">
          <button class="fold" aria-expanded={isOpen('smart-profile')} aria-controls={isOpen('smart-profile') ? 'rr-smart-profile' : undefined} onclick={() => fold('smart-profile')}><Icon name="chevron" /><span class="t">{t('Also correct the subscription’s rules')}</span></button>
          <button class="switch" role="switch" aria-label={t('Also correct the subscription’s rules')} aria-checked={s.core.smart_profile_rules}
            onclick={() => app.stageRules((x) => (x.core.smart_profile_rules = !x.core.smart_profile_rules))}></button>
        </div>
        {#if isOpen('smart-profile')}<div class="d" id="rr-smart-profile">{t('By default only sites no rule chose a route for are learned. With this on, Kova also fixes sites the subscription’s own rules send the wrong way, for example a games list sent direct where it is blocked. Your own rules and presets are never changed.')}</div>{/if}
      </div>
    {/if}
    {#if s.core.smart_routing}
      <!-- No arrow: the sentence is a privacy warning and stays in view. -->
      <div class="prow">
        <div class="line">
          <span class="fold plain"><span class="t">{t('Send sites that refuse the proxy direct')}</span></span>
          <button class="switch" role="switch" aria-label={t('Send sites that refuse the proxy direct')} aria-checked={s.core.smart_learn_direct}
            onclick={() => app.stageRules((x) => (x.core.smart_learn_direct = !x.core.smart_learn_direct))}></button>
        </div>
        <div class="d">{t('Off by default. To find out, Kova has to open such a site directly, and once sent direct the site sees your own address. With this off, Kova only moves blocked sites to the proxy.')}</div>
      </div>
    {/if}
    {#if suggestions.length}
      <div class="eyebrow sugg-h">{t('Suggestions')}</div>
      <div class="suggestions">
        {#each suggestions as sg (sg.domain)}
          <div class="sugg">
            <div class="sugg-text">
              <b class="mono">{sg.domain}</b>
              <span>{sg.route === 'proxy'
                ? t('is blocked direct ({why}), but {rule} sends it there. It opens through the proxy.', { why: REASON[sg.reason] ?? sg.reason, rule: culprit(sg) })
                : t('doesn’t open through the proxy ({why}), but {rule} sends it there. It opens directly.', { why: REASON[sg.reason] ?? sg.reason, rule: culprit(sg) })}</span>
            </div>
            <div class="ops">
              <button class="btn primary small-btn" onclick={() => acceptSuggestion(sg)}
                title={sg.origin === 'user' ? t('Adds a rule for this site right above yours') : t('Adds a rule of your own for this site')}>{t('Add exception')}</button>
              <button class="btn ghost small-btn" onclick={() => dismissSuggestion(sg)}>{t('Dismiss')}</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {#if learned.length}
      <div class="twrap list">
        <table>
          <thead><tr><th>{t('Site')}</th><th>{t('Now goes')}</th><th>{t('Why')}</th><th>{t('Learned')}</th><th></th></tr></thead>
          <tbody>
            {#each learned as l (l.domain)}
              <tr>
                <td class="mono small selectable">{l.domain}</td>
                <td><span class="pol {l.route === 'proxy' ? 'proxy' : 'direct'}">{l.route === 'proxy' ? t('Through the proxy') : t('Direct')}</span></td>
                <td class="muted">{REASON[l.reason] ?? l.reason}</td>
                <td class="muted">{fmtAgo(l.learned_at)}</td>
                <td class="r"><div class="ops">
                  <button class="btn ghost small-btn" onclick={() => keep(l.domain)} title={t('Make it one of your rules, kept for good')}>{t('Keep')}</button>
                  <button class="x" onclick={() => forget(l.domain)} aria-label={t('Forget {domain}', { domain: l.domain })}><Icon name="x" /></button>
                </div></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="actions gap"><button class="btn ghost danger" onclick={clearAll}>{t('Forget all')}</button></div>
    {:else if s.core.smart_routing}
      <p class="hint">{t('Nothing learned yet. A site appears here when Kova finds it works only the other way.')}</p>
    {/if}
  </section>

  <p class="hint order">{t('Order: your rules → torrent clients → presets → local network goes direct → everything else uses')} <b>{s.core.presets.includes('proxy-blocked-ru') ? 'DIRECT' : 'Proxy'}</b>. {t('Global and Direct modes skip rules entirely.')}</p>
{/if}


{#if routingApps}<AppRules onclose={() => (routingApps = false)} />{/if}

<style>
  .pending { position: sticky; top: 0; z-index: 3; display: flex; gap: 12px; align-items: center; justify-content: space-between; flex-wrap: wrap; margin-bottom: 12px; padding: 10px 12px; border: 1px solid var(--accent); border-radius: 8px; background: var(--sunk); font-size: 13px; }
  .presets { margin: 0 0 12px; }
  .presets .card-h { margin-bottom: 8px; }
  /* Small tiles side by side, so the presets take as little height as possible. align-items: start
     keeps a tile that is opened from stretching the ones next to it: it grows alone. The
     min() lets a single column shrink below 250 px on very narrow windows instead of overflowing. */
  .pgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(250px, 100%), 1fr)); gap: 8px; align-items: start; }
  /* A closed tile is one line: arrow and title on the left, the control at the end of the same line. */
  .tile.card { padding-block: 0; padding-inline: 12px; }
  .tile .line { min-height: 0; padding-block: 4px; }
  :global(.app.mobile) .tile .line { padding-block: 0; }
  /* A long title is cut with an ellipsis (the whole text is in the button's tooltip) so a tile stays one line. */
  .tile .t { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* A phone has one column and room for two lines, so a long title is read in full, not cut with an ellipsis. */
  :global(.app.mobile) .tile .t { overflow: visible; text-overflow: clip; white-space: normal; }
  .tile .d { margin: 0 0 8px; }
  .tile .link { margin-block: 0 8px; }
  .prow + .prow { border-top: 1px solid var(--line); }
  .line { display: flex; align-items: center; gap: 12px; min-height: 42px; }
  /* The 4 px of padding is undone by the negative margin, so the arrow sits on the card's edge and
     the title, descriptions, link and narrow torrent buttons all start 20 px in, in both directions. */
  .fold { flex: 1 1 140px; display: flex; align-items: center; gap: 6px; min-width: 0; min-height: 32px; border: 0; background: none; padding-block: 2px; padding-inline: 4px; margin-inline-start: -4px; color: inherit; font: inherit; cursor: pointer; border-radius: 6px; text-align: start; }
  .fold :global(.i) { width: 14px; height: 14px; color: var(--muted); transition: transform .15s; flex: none; }
  .fold[aria-expanded='true'] :global(.i) { transform: rotate(90deg); }
  .fold[aria-expanded='false'] :global(.i):dir(rtl) { transform: scaleX(-1); }
  @media (hover: hover) { .fold:hover :global(.i) { color: var(--fg); } }
  :global(.app.mobile) .fold { min-height: 40px; }
  /* A row with nothing to unfold keeps the title where the others have it: the arrow's width
     and gap (14 + 6) on top of the 4 px every .fold has at its start. */
  .fold.plain { cursor: default; padding-inline-start: 24px; }
  .t { font-weight: 500; font-size: 14px; }
  /* The explanation and the warnings sit under the title, in line with it. */
  .d { font-size: 12px; color: var(--muted); margin: 0 0 10px; padding-inline-start: 20px; }
  .link { display: block; margin: 0 0 10px; margin-inline-start: 20px; border: 0; background: none; padding: 0; color: var(--fg); font-size: 12px; text-decoration: underline; text-underline-offset: 2px; }
  .add { display: grid; grid-template-columns: 200px minmax(0, 1fr) 200px auto; gap: 8px; }
  .ai { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 10px; font-size: 13px; color: var(--muted); }
  .ai .input { width: auto; }
  @container (max-width: 760px) { .add { grid-template-columns: 1fr 1fr; } }
  .list { margin-top: 12px; }
  .small { font-size: 12px; }
  .grip-cell { width: 1%; padding-inline-end: 0; }
  .grip { border: 0; background: none; color: var(--faint); width: 22px; height: 26px; border-radius: 5px; display: grid; place-items: center; cursor: grab; touch-action: none; }
  .grip:hover { color: var(--fg); background: var(--sunk); }
  .grip:active { cursor: grabbing; }
  .grip :global(.i) { width: 16px; height: 16px; stroke-width: 2.8; }
  tr.dragging td { background: var(--accent-soft); }
  tr.dragging { box-shadow: 0 6px 20px rgb(0 0 0 / .35); position: relative; z-index: 2; }
  tr.moving { transition: transform .14s ease; }
  tbody:has(tr.dragging) { user-select: none; cursor: grabbing; }
  .hint :global(.inline-grip) { width: 13px; height: 13px; stroke-width: 3; vertical-align: -2px; }
  .apply { margin-top: 12px; border-top: 1px solid var(--line); }
  .order { margin-top: 14px; }
  /* The torrent tile is wider than the others so the four choices sit on its title line. It
     takes the whole row where the grid has fewer than three columns (a span of two would then
     make an extra column and overflow). */
  .tor-tile { grid-column: 1 / -1; }
  @container (min-width: 770px) { .tor-tile { grid-column: span 2; } }
  /* Squeezed (or with longer labels), the choices drop under the title instead of overflowing;
     the gap keeps them off the description and the border. */
  .tor { flex-wrap: wrap; gap: 8px 12px; }
  .tor .seg { flex: none; margin-inline-start: auto; padding: 2px; }
  .tor .seg button { padding: 2px 10px; }
  @container (max-width: 560px) {
    .tor .seg { flex: 1 1 100%; margin: 0; margin-inline-start: 20px; max-width: calc(100% - 20px); }
  }
  .warn-text { color: var(--warn); }
  .smart { margin-top: 12px; }
  .sugg-h { margin: 6px 0 8px; }
  .suggestions { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
  .sugg { display: flex; gap: 12px; align-items: center; justify-content: space-between; flex-wrap: wrap; padding: 10px 12px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--sunk); }
  .sugg-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1 1 260px; font-size: 13px; }
  .sugg-text span { color: var(--muted); }
  .smart .card-h { gap: 8px; }
  .smart .card-h .fold { flex: 0 1 auto; }
  /* The heading row sets its own height on desktop; on phones the arrow keeps its tap size. */
  :global(.app:not(.mobile)) .smart .card-h .fold { min-height: 0; }
  .aside-sw { margin-inline-start: auto; }
  .ops { display: flex; gap: 4px; justify-content: flex-end; align-items: center; }
  .small-btn { height: 26px; padding: 0 8px; font-size: 12px; }
  .gap { margin-top: 10px; }
  .order b { color: var(--fg); font-weight: 500; }
</style>
