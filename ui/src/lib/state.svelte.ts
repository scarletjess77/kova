// Global reactive state, fed by commands and the engine's event stream.

import { listen } from '@tauri-apps/api/event';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { api, errText, type EngineEvent, type NodeView, type Profile, type ProxySnapshot, type Settings, type Status } from './api';
import { i18n, resolveLang, t, tn } from './i18n.svelte';

export type View = 'dashboard' | 'proxies' | 'profiles' | 'connections' | 'rules' | 'check' | 'logs' | 'settings';
export interface LogLine { id: number; time: string; source: string; level: string; message: string }

const HISTORY = 60;
const MAX_LOGS = 1500;

class AppState {
  ready = $state(false);
  view = $state<View>('dashboard');
  status = $state<Status>({ running: false, busy: false, core: null, sidecar: false, since: null, error: null, mixed_port: 7890, bridged: [], dropped: [], tun_active: false, warning: null });
  settings = $state<Settings | null>(null);
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

  /** A newer Kova release, once found. */
  update = $state.raw<Update | null>(null);
  /** Download progress 0..1 while installing an update; `null` when idle. */
  updateProgress = $state<number | null>(null);
  checkingUpdate = $state(false);

  /** The "Add profile" dialog, openable from any view. */
  importOpen = $state(false);

  toast = $state<string | null>(null);
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  coresVersion = $state(0);
  /** The system's display language, used when the language setting is "system". */
  private systemLang = navigator.language;

  get activeProfile(): Profile | undefined {
    return this.profiles.find((p) => p.id === this.settings?.active_profile);
  }

  async init() {
    const snap = await api.getState();
    this.status = snap.status;
    this.settings = snap.settings;
    this.profiles = snap.profiles;
    this.dataDir = snap.data_dir;
    this.systemLang = await api.systemLanguage().catch(() => navigator.language);
    this.applyTheme();
    await listen<EngineEvent>('engine', (e) => this.onEvent(e.payload));
    await listen<string>('toast', (e) => this.notify(e.payload));
    await listen<Settings>('settings-changed', (e) => { this.settings = e.payload; this.applyTheme(); });
    await listen('cores-changed', () => { this.coresVersion++; });
    matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => this.applyTheme());
    this.ready = true;
    void this.refreshProxies();
    if (this.settings?.check_updates) setTimeout(() => void this.checkForUpdate(true), 5000);
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
        if (s.error && !s.busy && s.error !== oldError) this.notify(s.error);
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
      case 'delays':
        this.delays = { ...this.delays, ...e.delays };
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

  /** Apply the theme and language settings to the page. */
  applyTheme() {
    const theme = this.settings?.theme ?? 'dark';
    const light = theme === 'light' || (theme === 'system' && matchMedia('(prefers-color-scheme: light)').matches);
    if (light) document.documentElement.setAttribute('data-theme', 'light');
    else document.documentElement.removeAttribute('data-theme');
    i18n.lang = resolveLang(this.settings?.language, this.systemLang);
    document.documentElement.lang = i18n.lang;
  }

  async toggleConnection() {
    if (this.status.busy) return;
    if (this.status.running) await this.run(() => api.disconnect());
    else await this.run(() => api.connect());
  }

  async checkForUpdate(silent = false) {
    this.checkingUpdate = true;
    try {
      this.update = await check();
      if (!silent) this.notify(this.update ? t('Kova {version} is available', { version: this.update.version }) : t('Kova is up to date'));
    } catch (e) {
      if (!silent) this.notify(t('Could not check for updates: {error}', { error: errText(e) }));
    }
    this.checkingUpdate = false;
  }

  /** Download, install and restart. Disconnects first: the installer closes Kova
   *  without the usual shutdown, which would leave the system proxy pointing at it. */
  async installUpdate() {
    const u = this.update;
    if (!u || this.updateProgress !== null) return;
    if (this.status.running) await this.run(() => api.disconnect());
    let total = 0;
    let got = 0;
    this.updateProgress = 0;
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === 'Started') total = ev.data.contentLength ?? 0;
        else if (ev.event === 'Progress') {
          got += ev.data.chunkLength;
          this.updateProgress = total ? got / total : 0;
        } else this.updateProgress = 1;
      });
      await relaunch();
    } catch (e) {
      this.notify(t('The update failed: {error}', { error: errText(e) }));
      this.updateProgress = null;
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
    const targets = names ?? Object.keys(this.proxies.proxies);
    this.testing = new Set([...this.testing, ...targets]);
    let res: Record<string, number | null> | undefined;
    try {
      res = await api.testDelays(names);
    } catch (e) {
      if (!silent) this.notify(errText(e));
    }
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
}

function normaliseLevel(l: string): string {
  const x = l.toLowerCase();
  if (x.startsWith('warn')) return 'warning';
  if (x.startsWith('err') || x === 'fatal' || x === 'panic') return 'error';
  if (x === 'debug' || x === 'trace') return 'debug';
  return 'info';
}

export const app = new AppState();
