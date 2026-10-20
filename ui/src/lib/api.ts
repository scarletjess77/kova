// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs.
// Types mirror the serde output of kova-core / kova-engine.

import { invoke } from '@tauri-apps/api/core';

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
}

export interface Settings {
  core: RuntimeOptions;
  system_proxy: boolean;
  proxy_bypass: string;
  active_profile: string | null;
  connect_on_launch: boolean;
  start_minimized: boolean;
  launch_at_login: boolean;
  auto_update_cores: boolean;
  theme: 'dark' | 'light' | 'system';
  language: string;
  xray_selection: string | null;
  selections: Record<string, string>;
  check_sites: string[];
  check_updates: boolean;
}

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

export interface UserInfo { upload: number; download: number; total: number; expire: number | null }

export type Source =
  | { kind: 'remote'; url: string; user_agent: string | null; update_hours: number; via_proxy: boolean }
  | { kind: 'local' };

export interface Profile {
  id: string;
  name: string;
  source: Source;
  format: 'clash' | 'sing-box' | 'links';
  core: CorePreference;
  node_count: number;
  usage: UserInfo | null;
  updated_at: number;
  warnings: string[];
  support_url: string | null;
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
  | { type: 'delays'; delays: Record<string, number | null> };

export interface Snapshot { status: Status; settings: Settings; profiles: Profile[]; data_dir: string }

export const api = {
  getState: () => invoke<Snapshot>('get_state'),
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
  systemLanguage: () => invoke<string>('system_language'),
};

/** Tauri rejects with the serialised error string; make sure we always show text. */
export function errText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
