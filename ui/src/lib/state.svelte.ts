// Global reactive state, fed by commands and the engine's event stream.

import { listen } from '@tauri-apps/api/event';
import { friendly } from './errors';
import { enableTvNavigation } from './tvnav';
import { api, COMBINED, errText, type CoreStatus, type EngineEvent, type NodeView, type Profile, type ProxySnapshot, type Settings, type Status, type UpdateInfo, type DeviceInfo } from './api';
import { i18n, isRtl, resolveLang, t, tn } from './i18n.svelte';
import { resolveTheme } from './themes';
import { preset, resolve, type Form, type Layout, type Preset } from './layout';
import { applyLook, type Look } from './look';
import { loadPicture, type Picture } from './pictures';

export type View = 'dashboard' | 'proxies' | 'profiles' | 'connections' | 'rules' | 'check' | 'logs' | 'settings';
export interface LogLine { id: number; time: string; source: string; level: string; message: string }

const HISTORY = 60;
const MAX_LOGS = 1500;

type Core = Settings['core'];

/** The fields of `core` that differ from `saved`, or null when none does. */
function coreChanges(saved: Core, core: Core): Partial<Core> | null {
  const changed: Record<string, unknown> = {};
  for (const k of Object.keys(core) as (keyof Core)[]) {
    if (JSON.stringify(core[k]) !== JSON.stringify(saved[k])) changed[k] = core[k];
  }
  return Object.keys(changed).length ? (changed as Partial<Core>) : null;
}

class AppState {
  ready = $state(false);
  view = $state<View>('dashboard');
  status = $state<Status>({ running: false, busy: false, core: null, sidecar: false, since: null, error: null, mixed_port: 7890, bridged: [], dropped: [], tun_active: false, warning: null });
  settings = $state<Settings | null>(null);
  /** Edits made on the Rules tab that are not applied yet, as the `core` settings they
   *  change. Any change to the core restarts the connection, so these wait for Apply and
   *  are then saved together, with a single restart. */
  rulesDraft = $state<Partial<Core> | null>(null);
  applyingRules = $state(false);
  profiles = $state<Profile[]>([]);
  dataDir = $state('');

  down = $state<number[]>(Array(HISTORY).fill(0));
  up = $state<number[]>(Array(HISTORY).fill(0));
  downTotal = $state(0);
  upTotal = $state(0);
  memory = $state(0);

  logs = $state<LogLine[]>([]);
  logsPaused = $state(false);
  private logSeq = 0;

  proxies = $state<ProxySnapshot>({ groups: [], proxies: {} });
  /** Protocol details of the active profile's nodes, by name. */
  nodes = $state<Record<string, NodeView>>({});
  delays = $state<Record<string, number | null>>({});
  testing = $state<Set<string>>(new Set());
  private delayRun = 0;

  /** A newer Kova release, once found. */
  update = $state.raw<UpdateInfo | null>(null);
  /** Download progress 0..1 while installing an update; `null` when idle. */
  updateProgress = $state<number | null>(null);
  checkingUpdate = $state(false);

  /** The "Add profile" dialog, openable from any view. */
  importOpen = $state(false);
  /** What a kova:// link asked to import, for the dialog to start with. */
  importLink = $state<string | null>(null);

  toast = $state<string | null>(null);
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  coresVersion = $state(0);
  /** Running on a phone (Android): no window, tray, system proxy or in-app updates. */
  mobile = $state(false);
  /** The operating system: "windows", "linux", "macos", "android", ... */
  platform = $state('');
  /** Android TV: navigated with a remote, no touch, usually no camera. */
  tv = $state(false);
  camera = $state(true);
  /** Android: battery saving leaves Kova alone (it may stop the VPN otherwise). */
  batteryFree = $state(true);
  /** Android: the phone's maker, lower case, for its own background settings. */
  maker = $state('');
  /** A connect or disconnect the user started is under way. On Android that includes
   *  the permission dialog and opening the VPN, before the engine reports busy. */
  toggling = $state(false);
  /** Phone: the page list, opened from the title bar, is showing. */
  navOpen = $state(false);
  /** Bumped whenever smart routing learns a site, so lists can reload. */
  learnedVersion = $state(0);
  /** Smart-routing suggestions waiting for an answer, for the Rules badge. */
  suggestionCount = $state(0);
  /** The system's display language, used when the language setting is "system". */
  private systemLang = navigator.language;

  // Android's back gesture steps back through the page's history, and only leaves the
  // app when there is none. So each step back should undo (opening a page, the menu,
  // a dialog) adds a history entry together with how to undo it.
  private backSteps: (() => void)[] = [];
  /** Pops caused by dropBack, still to come, which mean nothing to undo. */
  private ignorePops = 0;
  /** Entries pushed while such a pop was still on its way. history.back() is
   *  asynchronous: an entry pushed before it lands would be the one it removes. */
  private deferredPushes = 0;
  private readonly closeNavStep = () => {
    this.navOpen = false;
  };

  /** Make the next back gesture run `undo`. */
  pushBack(undo: () => void) {
    if (!this.mobile) return;
    this.backSteps.push(undo);
    if (this.ignorePops > 0) this.deferredPushes++;
    else history.pushState(null, '');
  }

  /** Forget a step that was undone some other way (a dialog closed with its button). */
  dropBack(undo: () => void) {
    const i = this.backSteps.lastIndexOf(undo);
    if (i < 0) return;
    this.backSteps.splice(i, 1);
    if (this.deferredPushes > 0) {
      // Its entry was never pushed.
      this.deferredPushes--;
      return;
    }
    this.ignorePops++;
    history.back();
  }

  private onPopState = () => {
    if (this.ignorePops > 0) {
      this.ignorePops--;
      if (this.ignorePops === 0) {
        for (; this.deferredPushes > 0; this.deferredPushes--) history.pushState(null, '');
      }
      return;
    }
    this.backSteps.pop()?.();
  };

  /** Show a page; back returns to the one before. */
  go(view: View) {
    const from = this.view;
    if (this.navOpen && view === from) {
      // The page already showing: just close the menu.
      this.closeNav();
      return;
    }
    if (this.navOpen) {
      // Chosen from the menu: the menu's step becomes the page's.
      this.navOpen = false;
      const i = this.backSteps.lastIndexOf(this.closeNavStep);
      if (i >= 0) this.backSteps[i] = () => (this.view = from);
    } else if (view !== from) {
      this.pushBack(() => (this.view = from));
    }
    this.view = view;
  }

  openNav() {
    this.navOpen = true;
    this.pushBack(this.closeNavStep);
  }

  closeNav() {
    this.navOpen = false;
    this.dropBack(this.closeNavStep);
  }

  /** The cores and their versions, kept between visits to Settings so the card doesn't
   * fill in (and push the cards below it) every time it opens. */
  cores = $state<CoreStatus[]>([]);

  /** Blob URLs of the look's kept pictures, by file name. */
  pictureUrls = $state<Record<string, string>>({});
  private loadingPictures = new Set<string>();

  /** A kept picture's URL; starts loading it the first time and gives undefined till then. */
  pictureUrl(file: string | undefined): string | undefined {
    if (!file) return undefined;
    const url = this.pictureUrls[file];
    if (url || this.loadingPictures.has(file)) return url;
    this.loadingPictures.add(file);
    loadPicture(file).then(
      (u) => { this.pictureUrls = { ...this.pictureUrls, [file]: u }; },
      () => { /* gone from the folder: nothing to show */ },
    );
    return undefined;
  }

  /** The picture behind the window now: the open tab's own, or the one behind every tab. */
  get backdrop(): Picture | undefined {
    const p = this.settings?.look?.pictures ?? {};
    return p[`tab-${this.view}` as keyof typeof p] ?? p[this.mobile ? 'app-phone' : 'app'];
  }

  /** Custom CSS switched off until Kova restarts (Ctrl+Shift+K). */
  cssOff = $state(false);
  /** New custom CSS on trial: undone unless kept before the countdown ends. */
  cssTrial = $state<{ previous: string | null; left: number } | null>(null);
  private cssTimer: ReturnType<typeof setInterval> | undefined;

  tryCss(css: string) {
    const previous = this.cssTrial ? this.cssTrial.previous : (this.settings?.look?.css ?? null);
    void this.saveLook((l) => { if (css.trim()) l.css = css; else delete l.css; });
    this.cssTrial = { previous, left: 15 };
    clearInterval(this.cssTimer);
    this.cssTimer = setInterval(() => {
      if (!this.cssTrial) return clearInterval(this.cssTimer);
      if (this.cssTrial.left <= 1) this.undoCss();
      else this.cssTrial = { ...this.cssTrial, left: this.cssTrial.left - 1 };
    }, 1000);
  }

  keepCss() {
    clearInterval(this.cssTimer);
    this.cssTrial = null;
  }

  undoCss() {
    clearInterval(this.cssTimer);
    const previous = this.cssTrial?.previous ?? null;
    this.cssTrial = null;
    void this.saveLook((l) => { if (previous) l.css = previous; else delete l.css; }, t('Custom CSS undone'));
  }

  toggleCssOff() {
    this.cssOff = !this.cssOff;
    this.applyTheme();
    this.notify(this.cssOff ? t('Custom CSS off until Kova restarts') : t('Custom CSS on'));
  }

  /** The text colour was changed so it stays readable on the user's card colour. */
  lookAdjusted = $state(false);

  /** Shows a change to the look at once without saving it (while a slider moves). */
  previewLook(patch: (l: Look) => void) {
    if (!this.settings) return;
    const look: Look = JSON.parse(JSON.stringify(this.settings.look ?? {}));
    patch(look);
    this.settings = { ...this.settings, look };
    this.applyTheme();
  }

  saveLook(patch: (l: Look) => void, ok?: string) {
    return this.saveSettings((x) => {
      const look: Look = { ...(x.look ?? {}) };
      patch(look);
      x.look = look;
    }, ok);
  }

  /** Arranging tabs and Dashboard cards. */
  editingLayout = $state(false);

  /** Phones and desktops each keep their own layout. */
  get form(): Form {
    return this.mobile ? 'phone' : 'desktop';
  }

  get layout(): Layout {
    return resolve(this.settings?.layout?.[this.form], this.form);
  }

  saveLayout(l: Layout) {
    const form = this.form;
    return this.saveSettings((x) => { x.layout = { ...(x.layout ?? {}), [form]: l }; });
  }

  applyPreset(p: Preset) {
    return this.saveLayout(preset(p, this.form));
  }

  get activeProfile(): Profile | undefined {
    const id = this.settings?.active_profile;
    if (id === COMBINED) return this.combinedProfile;
    return this.profiles.find((p) => p.id === id);
  }

  /** The profiles in the combined list. */
  get combinedMembers(): Profile[] {
    const skip = this.settings?.combined_skip ?? [];
    return this.profiles.filter((p) => !skip.includes(p.id));
  }

  /** The combined list, shaped like a profile for the places that show the active one. */
  get combinedProfile(): Profile {
    const members = this.combinedMembers;
    return {
      id: COMBINED, name: t('All profiles'), source: { kind: 'local' }, format: 'links', core: 'auto',
      node_count: members.reduce((n, p) => n + p.node_count, 0), usage: null, updated_at: 0, warnings: [], support_url: null, announce: null,
    };
  }

  private async openImportLink() {
    const link = await api.takeImportLink().catch(() => null);
    if (!link) return;
    this.importLink = link;
    this.importOpen = false;
    // Reopened on the next tick so a dialog already showing starts over with the link.
    setTimeout(() => (this.importOpen = true));
  }

  async countSuggestions() {
    this.suggestionCount = (await api.smartSuggestions().catch(() => [])).length;
  }

  async init() {
    const snap = await api.getState();
    this.status = snap.status;
    this.settings = snap.settings;
    this.profiles = snap.profiles;
    void this.countSuggestions();
    this.dataDir = snap.data_dir;
    this.platform = await api.platform().catch(() => '');
    this.mobile = ['android', 'ios'].includes(this.platform);
    if (this.mobile) {
      await this.refreshDevice();
      if (this.tv) enableTvNavigation();
    }
    this.systemLang = await api.systemLanguage().catch(() => navigator.language);
    this.applyTheme();
    await listen<EngineEvent>('engine', (e) => this.onEvent(e.payload));
    await listen<string>('toast', (e) => this.notify(friendly(e.payload)));
    await listen<Settings>('settings-changed', (e) => { this.settings = e.payload; this.applyTheme(); });
    await listen('cores-changed', () => { this.coresVersion++; });
    // A kova:// link, such as "Add to Kova" on a subscription page. One that started
    // Kova is waiting already.
    await listen('import-link', () => void this.openImportLink());
    void this.openImportLink();
    matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => this.applyTheme());
    window.addEventListener('popstate', this.onPopState);
    // Hidden (tray, background app), the engine stops sending per-second figures.
    document.addEventListener('visibilitychange', () => {
      void api.pageVisible(!document.hidden).catch(() => {});
      // Back from Android's battery settings: see what was chosen there.
      if (!document.hidden && this.mobile) void this.refreshDevice();
    });
    // A crash closed Kova last time: say so once, so it can be reported.
    if (await api.crashNotice().catch(() => false)) this.notify(t('Kova closed unexpectedly last time. Copy report in Logs includes what happened.'));
    this.ready = true;
    void this.refreshProxies();
    if (this.settings?.check_updates) setTimeout(() => void this.checkForUpdate(true), 5000);
    // Kova often runs in the tray for weeks: look again once a day, and say so when a
    // new version turns up.
    setInterval(async () => {
      if (!this.settings?.check_updates || this.update) return;
      const found = await this.checkForUpdate(true);
      if (found) this.notify(t('Kova {version} is available', { version: found.version }));
    }, 24 * 3600 * 1000);
  }

  private onEvent(e: EngineEvent) {
    switch (e.type) {
      case 'status': {
        const { type: _, ...s } = e;
        const was = this.status.running;
        const oldError = this.status.error;
        this.status = s;
        if (was !== s.running) void this.refreshProxies();
        // Measure every node right after connecting so latency is visible and url-test groups settle.
        if (!was && s.running) setTimeout(() => void this.testDelays(undefined, true), 1200);
        if (s.error && !s.busy && s.error !== oldError) this.notify(friendly(s.error));
        break;
      }
      case 'traffic':
        this.down = [...this.down.slice(1), e.down];
        this.up = [...this.up.slice(1), e.up];
        this.downTotal = e.down_total;
        this.upTotal = e.up_total;
        break;
      case 'memory':
        this.memory = e.bytes;
        break;
      case 'log':
        if (this.logsPaused) break;
        this.logs.push({ id: this.logSeq++, time: new Date().toTimeString().slice(0, 8), source: e.source, level: normaliseLevel(e.level), message: e.message });
        if (this.logs.length > MAX_LOGS) this.logs.splice(0, this.logs.length - MAX_LOGS);
        break;
      case 'profiles':
        void api.getState().then((s) => { this.profiles = s.profiles; this.settings = s.settings; });
        void this.refreshProxies();
        break;
      case 'delays': {
        // Results stream in while a test runs: each server stops showing "…" as it answers.
        this.delays = { ...this.delays, ...e.delays };
        const still = new Set(this.testing);
        Object.keys(e.delays).forEach((n) => still.delete(n));
        this.testing = still;
        break;
      }
      case 'learned':
        this.learnedVersion++;
        this.notify(e.route === 'proxy'
          ? t('{domain} is blocked here; it now goes through the proxy', { domain: e.domain })
          : t('{domain} refuses the proxy; it now goes direct', { domain: e.domain }));
        break;
      case 'notice':
        this.notify(friendly(e.text));
        break;
      case 'suggested':
        this.learnedVersion++;
        void this.countSuggestions();
        if (e.origin === 'site-check') this.notify(t('{domain} opens only directly. See Rules → Smart routing', { domain: e.domain }));
        else this.notify(e.route === 'proxy'
          ? t('{domain} is blocked direct, but a rule sends it there. See Rules → Smart routing', { domain: e.domain })
          : t('{domain} doesn’t open through the proxy, but a rule sends it there. See Rules → Smart routing', { domain: e.domain }));
        break;
    }
  }

  notify(msg: string) {
    this.toast = msg;
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), 3200);
  }

  /** Run an action, turning failures into a toast. Returns undefined on failure. */
  async run<T>(fn: () => Promise<T>, ok?: string | ((v: T) => string)): Promise<T | undefined> {
    try {
      const v = await fn();
      if (ok) this.notify(typeof ok === 'string' ? ok : ok(v));
      return v;
    } catch (e) {
      this.notify(errText(e));
      return undefined;
    }
  }

  private saveQueue: Promise<void> = Promise.resolve();
  private saveSeq = 0;

  /** Apply `patch` on screen at once, then save. Saves run one after another, each
   *  sending everything changed so far, and only the newest result is shown, so quick
   *  successive changes can't undo each other on screen or restart the core in parallel. */
  async saveSettings(patch: (s: Settings) => void, ok?: string) {
    if (!this.settings) return;
    const next: Settings = JSON.parse(JSON.stringify(this.settings));
    patch(next);
    this.settings = next;
    this.applyTheme();
    const mine = ++this.saveSeq;
    const job = this.saveQueue.then(async () => {
      const toSend: Settings = JSON.parse(JSON.stringify(this.settings ?? next));
      const saved = await this.run(() => api.saveSettings(toSend), ok);
      if (mine !== this.saveSeq) return; // a newer save will settle the screen
      // On failure the engine may still have stored the change (it saves before
      // restarting the core), so show what it actually holds rather than guessing.
      this.settings = saved ?? (await api.getState().then((s) => s.settings).catch(() => toSend));
      this.applyTheme();
    });
    this.saveQueue = job.catch(() => {});
    await job;
  }

  /** The settings with the Rules edits that wait for Apply on top: what the Rules tab shows. */
  get ruleSettings(): Settings | null {
    const s = this.settings;
    if (!s || !this.rulesDraft) return s;
    return { ...s, core: { ...s.core, ...this.rulesDraft } };
  }

  /** Change settings from the Rules tab without saving: the change shows at once and
   *  waits for `applyRules`. Undoing every edit leaves nothing waiting. */
  stageRules(patch: (x: Settings) => void) {
    const saved = this.settings;
    const shown = this.ruleSettings;
    if (!saved || !shown) return;
    const next: Settings = JSON.parse(JSON.stringify(shown));
    patch(next);
    this.rulesDraft = coreChanges(saved.core, next.core);
  }

  discardRules() {
    this.rulesDraft = null;
  }

  /** Save the waiting Rules edits in one go, restarting the connection once. */
  async applyRules(notify = true) {
    const draft = this.rulesDraft;
    if (!draft || this.applyingRules) return;
    this.applyingRules = true;
    try {
      const changes = JSON.parse(JSON.stringify(draft)) as Partial<Core>;
      await this.saveSettings((x) => Object.assign(x.core, changes), notify ? t('Rules applied') : undefined);
    } finally {
      this.applyingRules = false;
      // What the engine now holds is no longer waiting. What it refused (a rule it can't
      // use, say) stays, along with anything edited meanwhile, to be fixed and applied again.
      const saved = this.settings;
      this.rulesDraft = saved ? coreChanges(saved.core, { ...saved.core, ...draft, ...this.rulesDraft }) : null;
    }
  }

  /** Apply the theme and language settings to the page. */
  async refreshDevice() {
    const d = await api.deviceInfo().catch(() => ({ tv: false, camera: true }) as DeviceInfo);
    this.tv = d.tv;
    this.camera = d.camera;
    this.batteryFree = d.batteryFree ?? true;
    this.maker = d.maker ?? '';
  }

  applyTheme() {
    const theme = resolveTheme(this.settings?.theme);
    if (theme.id === 'dark') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', theme.id);
    this.lookAdjusted = applyLook(this.settings?.look).adjusted;
    // The user's CSS goes last in the page, so it wins over Kova's own.
    let style = document.getElementById('kova-user-css');
    if (!style) {
      style = document.createElement('style');
      style.id = 'kova-user-css';
    }
    const css = this.cssOff ? '' : (this.settings?.look?.css ?? '');
    if (style.textContent !== css) style.textContent = css;
    document.head.appendChild(style);
    i18n.lang = resolveLang(this.settings?.language, this.systemLang);
    document.documentElement.lang = i18n.lang;
    document.documentElement.dir = isRtl(i18n.lang) ? 'rtl' : 'ltr';
  }

  async toggleConnection() {
    if (this.status.busy || this.toggling) return;
    this.toggling = true;
    try {
      if (this.status.running) await this.run(() => api.disconnect());
      else {
        // Rules waiting for Apply would otherwise be left out of the new connection.
        await this.applyRules(false);
        await this.run(() => api.connect());
      }
    } finally {
      this.toggling = false;
    }
  }

  async checkForUpdate(silent = false) {
    this.checkingUpdate = true;
    try {
      this.update = await api.checkUpdate();
      if (!silent) this.notify(this.update ? t('Kova {version} is available', { version: this.update.version }) : t('Kova is up to date'));
    } catch (e) {
      if (!silent) this.notify(t('Could not check for updates: {error}', { error: errText(e) }));
    }
    this.checkingUpdate = false;
    return this.update;
  }

  /** Download, install and restart (Kova disconnects first). */
  async installUpdate() {
    if (!this.update || this.updateProgress !== null) return;
    this.updateProgress = 0;
    const stop = await listen<number>('update-progress', (e) => (this.updateProgress = e.payload));
    try {
      await api.installUpdate();
      // Android's installer is now in front; should the user back out of it, the
      // button is there again.
      if (this.mobile) setTimeout(() => (this.updateProgress = null), 4000);
    } catch (e) {
      this.notify(t('The update failed: {error}', { error: errText(e) }));
      this.updateProgress = null;
    } finally {
      stop();
    }
  }

  async refreshProxies() {
    const id = this.settings?.active_profile;
    if (id) {
      api.profileNodes(id).then((list) => { this.nodes = Object.fromEntries(list.map((n) => [n.name, n])); }).catch(() => {});
    } else {
      this.nodes = {};
    }
    try {
      const snap = await api.getProxies();
      this.proxies = snap;
      const d: Record<string, number | null> = { ...this.delays };
      for (const [name, p] of Object.entries(snap.proxies)) if (p.delay != null) d[name] = p.delay;
      this.delays = d;
    } catch {
      /* no profile yet */
    }
  }

  async testDelays(names?: string[], silent = false) {
    const run = ++this.delayRun;
    const targets = names ?? Object.keys(this.proxies.proxies);
    this.testing = new Set([...this.testing, ...targets]);
    let res: Record<string, number | null> | undefined;
    try {
      res = await api.testDelays(names);
    } catch (e) {
      if (!silent) this.notify(errText(e));
    }
    // A newer test replaced this one: it owns the "…" marks and says the result.
    if (run !== this.delayRun) return;
    const still = new Set(this.testing);
    targets.forEach((n) => still.delete(n));
    this.testing = still;
    if (res) {
      this.delays = { ...this.delays, ...res };
      void this.refreshProxies();
      if (silent) return;
      const ok = Object.values(res).filter((v) => v != null) as number[];
      this.notify(ok.length ? tn(Object.keys(res).length, 'Tested {n} node · fastest {ms} ms', 'Tested {n} nodes · fastest {ms} ms', { ms: Math.min(...ok) }) : t('No node answered the latency test'));
    }
  }

  async select(group: string, name: string) {
    await this.run(() => api.selectProxy(group, name));
    await this.refreshProxies();
  }

  /** Follow a group chain to the node that actually carries traffic. */
  resolve(name: string, depth = 0): string {
    const g = this.proxies.groups.find((x) => x.name === name);
    if (!g || depth > 8) return name;
    return this.resolve(g.now || g.all[0] || name, depth + 1);
  }

  delayOf(name: string): number | null {
    return this.delays[this.resolve(name)] ?? this.proxies.proxies[this.resolve(name)]?.delay ?? null;
  }

  /** Tested and gave no answer, as opposed to not tested yet. */
  isDead(name: string): boolean {
    const k = this.resolve(name);
    return k in this.delays && this.delays[k] === null;
  }
}

function normaliseLevel(l: string): string {
  const x = l.toLowerCase();
  if (x.startsWith('warn')) return 'warning';
  if (x.startsWith('err') || x === 'fatal' || x === 'panic') return 'error';
  if (x === 'debug' || x === 'trace') return 'debug';
  return 'info';
}

export const app = new AppState();
