// Theme files (`.kovatheme`): the colour theme, the look (colours, font, size, shape,
// custom CSS), both layouts and the pictures themselves, in one JSON file to share.
// Reading one checks every value: a file from someone else is data, never trusted.

import { invoke } from '@tauri-apps/api/core';
import type { Settings } from './api';
import { CSS_LIMIT, FONTS, LIMITS, type FontId, type Look, type LookColors } from './look';
import { type Item, type Layout, type SavedLayouts } from './layout';
import { TAB_VIEWS, type Fit, type Picture, type Pos, type SlotId } from './pictures';
import { THEMES } from './themes';

export const THEME_FILE_VERSION = 1;
/** Bigger files are refused before reading: every picture at its limit fits well under it. */
export const THEME_FILE_LIMIT = 120 * 1024 * 1024;

export interface ThemeFile {
  kova_theme: number;
  name: string;
  created: number;
  theme?: Settings['theme'];
  look?: Look;
  layout?: SavedLayouts;
  /** Picture bytes in base64, by slot. */
  pictures?: Partial<Record<SlotId, string>>;
}

export interface ExportChoice { layout: boolean; pictures: boolean; css: boolean }

// ---- bytes ----

function toBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

function fromBase64(b64: string): Uint8Array {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

// ---- writing ----

/** The file's text for the current settings. */
export async function buildThemeFile(settings: Settings, name: string, choice: ExportChoice): Promise<string> {
  const look: Look = JSON.parse(JSON.stringify(settings.look ?? {}));
  if (!choice.css) delete look.css;
  const pictures: Partial<Record<SlotId, string>> = {};
  if (choice.pictures) {
    for (const [slot, pic] of Object.entries(look.pictures ?? {}) as [SlotId, Picture][]) {
      const buf = await invoke<ArrayBuffer>('look_picture', { name: pic.file });
      pictures[slot] = toBase64(new Uint8Array(buf));
    }
  } else {
    delete look.pictures;
  }
  const file: ThemeFile = {
    kova_theme: THEME_FILE_VERSION,
    name: name.trim() || 'Kova theme',
    created: Math.floor(Date.now() / 1000),
    theme: settings.theme,
    look,
    ...(choice.layout && settings.layout ? { layout: settings.layout } : {}),
    ...(Object.keys(pictures).length ? { pictures } : {}),
  };
  return JSON.stringify(file);
}

export const saveTextDocument = (path: string | null, name: string, text: string) =>
  invoke<boolean>('save_text_document', { path, name, text });

// ---- reading ----

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const num = (v: unknown, [lo, hi]: readonly [number, number]) => (typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : undefined);
const HEX = /^#[0-9a-f]{3}(?:[0-9a-f]{3})?$/i;
const SLOTS: SlotId[] = ['app', 'app-phone', 'sidebar', 'banner', ...TAB_VIEWS.map((v) => `tab-${v}` as SlotId)];

function cleanLook(v: unknown): Look {
  if (!isObj(v)) return {};
  const out: Look = {};
  if (isObj(v.colors)) {
    const c: LookColors = {};
    for (const k of ['accent', 'bg', 'panel', 'fg'] as const) {
      const x = v.colors[k];
      if (typeof x === 'string' && HEX.test(x)) c[k] = x;
    }
    if (Object.keys(c).length) out.colors = c;
  }
  if (typeof v.font === 'string' && v.font in FONTS) out.font = v.font as FontId;
  const scale = num(v.scale, LIMITS.scale);
  if (scale != null) out.scale = scale;
  const radius = num(v.radius, LIMITS.radius);
  if (radius != null) out.radius = radius;
  if (v.borders === false) out.borders = false;
  const glass = num(v.glass, LIMITS.glass);
  if (glass) out.glass = glass;
  if (v.density === 'compact') out.density = 'compact';
  if (typeof v.css === 'string' && v.css.trim() && v.css.length <= CSS_LIMIT) out.css = v.css;
  if (isObj(v.pictures)) {
    const pics: Partial<Record<SlotId, Picture>> = {};
    for (const slot of SLOTS) {
      const p = v.pictures[slot];
      if (!isObj(p)) continue;
      pics[slot] = {
        file: '',
        ...(['cover', 'contain', 'tile'].includes(p.fit as string) ? { fit: p.fit as Fit } : {}),
        ...(['center', 'top', 'bottom', 'left', 'right'].includes(p.pos as string) ? { pos: p.pos as Pos } : {}),
        ...(num(p.dim, [0, 90]) != null ? { dim: num(p.dim, [0, 90]) } : {}),
        ...(num(p.blur, [0, 20]) != null ? { blur: num(p.blur, [0, 20]) } : {}),
      };
    }
    if (Object.keys(pics).length) out.pictures = pics;
  }
  return out;
}

function cleanItems(v: unknown): Item[] | undefined {
  if (!Array.isArray(v)) return undefined;
  return v.filter((x) => isObj(x) && typeof x.id === 'string' && x.id.length < 40).map((x) => ({ id: (x as Item).id, ...((x as Item).hidden === true ? { hidden: true } : {}) }));
}

function cleanLayout(v: unknown): Layout | undefined {
  if (!isObj(v)) return undefined;
  const out: Layout = { tabs: (cleanItems(v.tabs) ?? []) as Layout['tabs'], cards: (cleanItems(v.cards) ?? []) as Layout['cards'] };
  const status = cleanItems(v.status);
  if (status) out.status = status as Layout['status'];
  if (isObj(v.proxies)) {
    const p = v.proxies;
    out.proxies = Object.fromEntries((['compact', 'protocol', 'transport', 'cores'] as const).filter((k) => typeof p[k] === 'boolean').map((k) => [k, p[k]]));
  }
  return out;
}

/** A theme file's contents, checked; throws `not-theme` or `newer` when it can't be used. */
export function readThemeFile(text: string): ThemeFile {
  let raw: unknown;
  try { raw = JSON.parse(text); } catch { throw new Error('not-theme'); }
  if (!isObj(raw) || typeof raw.kova_theme !== 'number') throw new Error('not-theme');
  if (raw.kova_theme > THEME_FILE_VERSION) throw new Error('newer');
  const file: ThemeFile = {
    kova_theme: raw.kova_theme,
    name: typeof raw.name === 'string' ? raw.name.slice(0, 80) : 'Kova theme',
    created: typeof raw.created === 'number' ? raw.created : 0,
  };
  if (typeof raw.theme === 'string' && (raw.theme === 'system' || THEMES.some((x) => x.id === raw.theme))) file.theme = raw.theme as Settings['theme'];
  file.look = cleanLook(raw.look);
  if (isObj(raw.layout)) {
    const layout: SavedLayouts = {};
    const d = cleanLayout(raw.layout.desktop), p = cleanLayout(raw.layout.phone);
    if (d) layout.desktop = d;
    if (p) layout.phone = p;
    if (d || p) file.layout = layout;
  }
  if (isObj(raw.pictures)) {
    const pics: Partial<Record<SlotId, string>> = {};
    for (const slot of SLOTS) {
      const b = raw.pictures[slot];
      if (typeof b === 'string' && b.length < 15 * 1024 * 1024 && file.look.pictures?.[slot]) pics[slot] = b;
    }
    file.pictures = pics;
  }
  // A picture setting without its picture has nothing to show.
  for (const slot of Object.keys(file.look.pictures ?? {}) as SlotId[]) {
    if (!file.pictures?.[slot]) delete file.look.pictures![slot];
  }
  return file;
}

/** Keeps each picture of `file` (Kova checks each one's format and size) and returns the look with their new names. */
export async function keepThemePictures(file: ThemeFile): Promise<Look> {
  const look: Look = JSON.parse(JSON.stringify(file.look ?? {}));
  for (const [slot, b64] of Object.entries(file.pictures ?? {}) as [SlotId, string][]) {
    const bytes = fromBase64(b64);
    const name = await invoke<string>('look_picture_save', bytes, { headers: { slot } });
    look.pictures![slot]!.file = name;
  }
  return look;
}
