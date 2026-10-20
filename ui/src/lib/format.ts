import { t, tn } from './i18n.svelte';

export function fmtBytes(b: number): string {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  const v = i === 0 ? Math.round(b) : b < 10 ? b.toFixed(2) : b < 100 ? b.toFixed(1) : Math.round(b);
  return `${v} ${u[i]}`;
}

export const fmtSpeed = (b: number) => `${fmtBytes(b)}/s`;

export function fmtDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
  const p = (n: number) => String(n).padStart(2, '0');
  return (h ? `${p(h)}:` : '') + `${p(m)}:${p(x)}`;
}

export function fmtAgo(unixSecs: number): string {
  if (!unixSecs) return t('never');
  const d = Date.now() / 1000 - unixSecs;
  if (d < 90) return t('just now');
  if (d < 3600) return t('{n} min ago', { n: Math.round(d / 60) });
  if (d < 86400) return t('{n} h ago', { n: Math.round(d / 3600) });
  const days = Math.round(d / 86400);
  return days === 1 ? t('yesterday') : tn(days, '{n} day ago', '{n} days ago');
}

export function fmtDate(unixSecs: number): string {
  return new Date(unixSecs * 1000).toISOString().slice(0, 10);
}

export const daysUntil = (unixSecs: number) => Math.ceil((unixSecs * 1000 - Date.now()) / 86400000);

export type LatClass = 'ok' | 'warn' | 'bad' | 'none';
export function latClass(d: number | null | undefined): LatClass {
  if (d == null) return 'none';
  return d < 150 ? 'ok' : d < 400 ? 'warn' : 'bad';
}

/**
 * Two-letter country code from a node name. Handles flag emoji (🇳🇱), leading codes
 * ("NL Amsterdam", "NL-01") and common country names in English and Russian.
 */
const NAMES: Record<string, string> = {
  netherlands: 'NL', нидерланды: 'NL', germany: 'DE', германия: 'DE', finland: 'FI', финляндия: 'FI',
  poland: 'PL', польша: 'PL', sweden: 'SE', швеция: 'SE', 'united kingdom': 'GB', britain: 'GB', великобритания: 'GB',
  turkey: 'TR', türkiye: 'TR', турция: 'TR', 'united states': 'US', usa: 'US', сша: 'US', japan: 'JP', япония: 'JP',
  singapore: 'SG', сингапур: 'SG', france: 'FR', франция: 'FR', estonia: 'EE', эстония: 'EE', latvia: 'LV', латвия: 'LV',
  lithuania: 'LT', литва: 'LT', kazakhstan: 'KZ', казахстан: 'KZ', russia: 'RU', россия: 'RU', canada: 'CA', канада: 'CA',
  switzerland: 'CH', швейцария: 'CH', austria: 'AT', австрия: 'AT', 'hong kong': 'HK', гонконг: 'HK',
};

export function countryCode(name: string): string | null {
  const flag = [...name].map((c) => c.codePointAt(0)!).filter((cp) => cp >= 0x1f1e6 && cp <= 0x1f1ff);
  if (flag.length >= 2) return String.fromCharCode(flag[0] - 0x1f1e6 + 65, flag[1] - 0x1f1e6 + 65);
  const lead = name.trim().match(/^([A-Z]{2})(?:[\s\-_|·.]|\d|$)/);
  if (lead) return lead[1];
  const lower = name.toLowerCase();
  for (const [k, v] of Object.entries(NAMES)) if (lower.includes(k)) return v;
  return null;
}

/** Node name without a leading flag emoji. */
export function displayName(name: string): string {
  return name.replace(/^[\u{1F1E6}-\u{1F1FF}]{2}\s*/u, '').trim() || name;
}
