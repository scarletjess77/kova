// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs.
// Types mirror the serde output of kova-core / kova-engine.

import { invoke } from '@tauri-apps/api/core';
import { friendly } from './errors';

export type CoreKind = 'mihomo' | 'xray';
export type CorePreference = 'auto' | 'mihomo' | 'xray';
export type Mode = 'rule' | 'global' | 'direct';
export type Preset = 'block-ads' | 'direct-ru' | 'direct-cn' | 'direct-ir' | 'proxy-blocked-ru';
export type GeoSource = { kind: 'default' } | { kind: 'runetfreedom' } | { kind: 'custom'; geoip: string; geosite: string };
export interface CheckSite { name: string; url: string; group: string }
export interface Probe {
  outcome: 'ok' | 'refused' | 'http-error' | 'timeout' | 'reset' | 'tls' | 'dns' | 'connect-failed';
  status: number | null;
  ms: number | null;
  detail: string | null;
}
export interface SiteResult { url: string; via_kova: Probe | null; direct: Probe }
export interface ServiceView { supported: boolean; installed: boolean; running: boolean; version: string | null; app_version: string; core_version: string | null; outdated: boolean }
export interface GeoMarker { source: string; verified: boolean; updated_at: number }
export interface GeoInfo { installed: GeoMarker | null; files: { name: string; size: number; categories: number }[]; missing: string[] }
export type RuleKind = 'domain' | 'domain-suffix' | 'domain-keyword' | 'geosite' | 'geoip' | 'ip-cidr' | 'process-name' | 'match';

export interface Rule { kind: RuleKind; value: string; target: string }

export interface RuntimeOptions {
  /** Kova's login on its local ports (user `kova`); on Android also on the proxy port. */
  local_password: string;
  mixed_port: number;
  controller_port: number;
  secret: string;
  allow_lan: boolean;
  ipv6: boolean;
  mode: Mode;
  log_level: 'debug' | 'info' | 'warning' | 'error' | 'silent';
  sniff: boolean;
  tun: { enabled: boolean; stack: 'mixed' | 'system' | 'gvisor'; auto_route: boolean; strict_route: boolean; mtu: number };
  dns: { fake_ip: boolean; nameservers: string[]; direct_nameservers: string[]; override_profile: boolean };
  presets: Preset[];
  custom_rules: Rule[];
  apply_rules_to_clash_profiles: boolean;
  xray_metrics_port: number;
  geodata: GeoSource;
  smart_routing: boolean;
  smart_profile_rules: boolean;
  smart_learn_direct: boolean;
  /** Beta: split the TLS handshake and add UDP noise on connections to the servers. */
  anti_dpi: boolean;
}

export type Route = 'proxy' | 'direct';
/** A site smart routing learned. `reason` is a site-check outcome such as "reset". */
export interface Learned { domain: string; route: Route; reason: string; learned_at: number }
/** Where a rule comes from: the user's own, a preset, or the imported subscription. */
export type RuleOrigin = 'user' | 'preset' | 'profile' | 'site-check';
/** A site that fails on the route a rule chose on purpose; `rule` as Mihomo names it. */
export interface Suggestion { domain: string; route: Route; reason: string; rule: string; origin: RuleOrigin; at: number }

export interface Settings {
  core: RuntimeOptions;
  system_proxy: boolean;
  proxy_bypass: string;
  active_profile: string | null;
  connect_on_launch: boolean;
  start_minimized: boolean;
  launch_at_login: boolean;
  auto_update_cores: boolean;
  theme: 'dark' | 'light' | 'system' | 'oled' | 'dim' | 'warm' | 'contrast' | 'nord' | 'catppuccin' | 'dracula' | 'gruvbox' | 'solarized';
  language: string;
  xray_selection: string | null;
  selections: Record<string, string>;
  check_sites: string[];
  check_updates: boolean;
  beta_updates: boolean;
  /** Android: which apps go through the VPN. */
  vpn_apps: { mode: VpnAppsMode; apps: string[] };
  /** Move to the fastest working node when the chosen one stops answering. */
  auto_switch: boolean;
  /** Random per-install ID sent to subscription servers. */
  device_id: string;
  /** Android: the main group's member to use on mobile data; null for the same as on Wi-Fi. */
  mobile_server: string | null;
}

export type VpnAppsMode = 'all' | 'only' | 'except';
/** One step of a diagnostic or privacy check; the interface words it by id and state. */
export interface Check { id: string; state: 'ok' | 'warn' | 'fail' | 'skipped'; detail: string | null; fix: string | null }
/** An app installed on the phone; `icon` is a data URL. */
export interface VpnApp { package: string; label: string; system: boolean; icon: string }

export interface Status {
  running: boolean;
  busy: boolean;
  core: CoreKind | null;
  sidecar: boolean;
  since: number | null;
  error: string | null;
  mixed_port: number;
  bridged: string[];
  dropped: [string, string][];
  tun_active: boolean;
  warning: string | null;
}

/** A newer Kova release. */
export interface UpdateInfo { version: string; date: string | null; body: string | null }

export interface UserInfo { upload: number; download: number; total: number; expire: number | null }

export type Source =
  | { kind: 'remote'; url: string; user_agent: string | null; update_hours: number; via_proxy: boolean }
  | { kind: 'local' };

export interface Profile {
  id: string;
  name: string;
  source: Source;
  format: 'clash' | 'sing-box' | 'xray' | 'links';
  core: CorePreference;
  node_count: number;
  usage: UserInfo | null;
  updated_at: number;
  warnings: string[];
  support_url: string | null;
  /** The provider's latest message, from the last update. */
  announce: string | null;
}

export interface NodeView {
  name: string;
  protocol: string;
  transport: string;
  server: string;
  port: number;
  mihomo: boolean;
  xray: boolean;
  link: string | null;
}

export interface ProxyGroup { name: string; type: string; now: string; all: string[] }
export interface ProxyInfo { name: string; type: string; udp: boolean; alive: boolean; delay: number | null }
export interface ProxySnapshot { groups: ProxyGroup[]; proxies: Record<string, ProxyInfo> }

export interface Connection {
  id: string; host: string; dst: string; port: string; network: string; process: string;
  rule: string; chains: string[]; upload: number; download: number; start: string;
}
export interface ConnectionsSnapshot { upload_total: number; download_total: number; connections: Connection[] }

export interface CoreStatus { core: CoreKind; installed: string | null; latest: string | null }
export interface ExitInfo { ip: string; country: string; country_code: string; city: string; network: string }

export type EngineEvent =
  | ({ type: 'status' } & Status)
  | { type: 'traffic'; up: number; down: number; up_total: number; down_total: number }
  | { type: 'log'; source: string; level: string; message: string }
  | { type: 'memory'; bytes: number }
  | { type: 'profiles' }
  | { type: 'delays'; delays: Record<string, number | null> }
  | { type: 'learned'; domain: string; route: Route; reason: string }
  | { type: 'notice'; text: string }
  | { type: 'suggested'; domain: string; route: Route; rule: string; origin: RuleOrigin };

export interface Snapshot { status: Status; settings: Settings; profiles: Profile[]; data_dir: string }

export const api = {
  getState: () => invoke<Snapshot>('get_state'),
  takeImportLink: () => invoke<string | null>('take_import_link'),
  connect: () => invoke<void>('connect'),
  disconnect: () => invoke<void>('disconnect'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  addProfile: (input: string, name?: string) => invoke<Profile>('add_profile', { input, name }),
  updateProfile: (id: string) => invoke<Profile>('update_profile', { id }),
  updateAllProfiles: () => invoke<[string, string][]>('update_all_profiles'),
  removeProfile: (id: string) => invoke<void>('remove_profile', { id }),
  setActiveProfile: (id: string) => invoke<void>('set_active_profile', { id }),
  editProfile: (id: string, patch: Partial<{ name: string; core: CorePreference; url: string; user_agent: string; update_hours: number; via_proxy: boolean }>) =>
    invoke<Profile>('edit_profile', { id, patch }),
  getProfileContent: (id: string) => invoke<string>('get_profile_content', { id }),
  saveProfileContent: (id: string, text: string) => invoke<void>('save_profile_content', { id, text }),
  profileNodes: (id: string) => invoke<NodeView[]>('profile_nodes', { id }),
  getProxies: () => invoke<ProxySnapshot>('get_proxies'),
  selectProxy: (group: string, name: string) => invoke<void>('select_proxy', { group, name }),
  testDelays: (names?: string[]) => invoke<Record<string, number | null>>('test_delays', { names }),
  getConnections: () => invoke<ConnectionsSnapshot>('get_connections'),
  closeConnection: (id: string) => invoke<void>('close_connection', { id }),
  closeAllConnections: () => invoke<void>('close_all_connections'),
  coreStatus: (checkLatest: boolean) => invoke<CoreStatus[]>('core_status', { checkLatest }),
  installCore: (core: CoreKind) => invoke<string>('install_core', { core }),
  exitInfo: () => invoke<ExitInfo>('exit_info'),
  geodataInfo: () => invoke<GeoInfo>('geodata_info'),
  defaultCheckSites: () => invoke<CheckSite[]>('default_check_sites'),
  checkSite: (url: string) => invoke<SiteResult>('check_site', { url }),
  serviceStatus: () => invoke<ServiceView>('service_status'),
  serviceInstall: () => invoke<void>('service_install'),
  serviceUninstall: () => invoke<void>('service_uninstall'),
  updateGeodata: () => invoke<GeoMarker>('update_geodata'),
  windowAction: (action: 'minimize' | 'maximize' | 'hide') => invoke<void>('window_action', { action }),
  quit: () => invoke<void>('quit'),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
  systemLanguage: () => invoke<string>('system_language'),
  platform: () => invoke<string>('platform'),
  deviceInfo: () => invoke<{ tv: boolean; camera: boolean }>('device_info'),
  diagnose: () => invoke<Check[]>('diagnose'),
  privacyCheck: () => invoke<Check[]>('privacy_check'),
  speedTest: () => invoke<{ through_kova: number | null; direct: number | null }>('speed_test'),
  lanAddress: () => invoke<string | null>('lan_address'),
  receiveStart: () => invoke<{ url: string }>('receive_start'),
  receiveStop: () => invoke<void>('receive_stop'),
  vpnApps: () => invoke<{ apps: VpnApp[] }>('vpn_apps'),
  openVpnSettings: () => invoke<void>('open_vpn_settings'),
  learnedSites: () => invoke<Learned[]>('learned_sites'),
  forgetLearned: (domain: string) => invoke<void>('forget_learned', { domain }),
  keepLearned: (domain: string) => invoke<Settings>('keep_learned', { domain }),
  clearLearned: () => invoke<void>('clear_learned'),
  smartSuggestions: () => invoke<Suggestion[]>('smart_suggestions'),
  acceptSuggestion: (domain: string) => invoke<Settings>('accept_suggestion', { domain }),
  dismissSuggestion: (domain: string) => invoke<void>('dismiss_suggestion', { domain }),
};

/** Tauri rejects with the serialised error string; make sure we always show text. */
/** An error from the engine, in plain words in the interface's language. */
export function errText(e: unknown): string {
  return friendly(typeof e === 'string' ? e : e instanceof Error ? e.message : String(e));
}
