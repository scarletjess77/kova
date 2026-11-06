// The user's own look, drawn over the chosen theme: colours, font, size, corners,
// borders, card transparency and density. Colours the user didn't pick come from the
// theme; the shades between them (lines, faded text, the soft accent) are worked out
// from the four the user sees. Saved in Kova's settings.

import type { Picture, SlotId } from './pictures';

export interface LookColors {
  accent?: string;
  bg?: string;
  panel?: string;
  fg?: string;
}

export type FontId = 'onest' | 'system' | 'vazir' | 'mono' | 'pixel';
export type Density = 'comfortable' | 'compact';

export interface Look {
  colors?: LookColors;
  font?: FontId;
  /** Interface size in percent, 90 to 120. */
  scale?: number;
  /** Corner roundness of cards in px, 0 to 20. */
  radius?: number;
  /** false: cards without their outline. */
  borders?: boolean;
  /** How much the background shows through cards, 0 to 60 percent. */
  glass?: number;
  density?: Density;
  /** Pictures by slot (see lib/pictures.ts). */
  pictures?: Partial<Record<SlotId, Picture>>;
}

export const FONTS: Record<FontId, string> = {
  onest: "'Onest Variable', 'Vazirmatn Variable', 'Segoe UI', system-ui, sans-serif",
  system: "system-ui, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif",
  vazir: "'Vazirmatn Variable', 'Segoe UI', system-ui, sans-serif",
  mono: "'JetBrains Mono Variable', ui-monospace, 'Cascadia Mono', Consolas, monospace",
  pixel: "'Pixelify Sans Variable', 'Onest Variable', system-ui, sans-serif",
};

export const LIMITS = { scale: [90, 120], radius: [0, 20], glass: [0, 60] } as const;
export const DEFAULT_RADIUS = 10;

/** Whether `look` changes anything at all. */
export function isEmpty(look: Look | null | undefined): boolean {
  if (!look) return true;
  const c = look.colors ?? {};
  return !c.accent && !c.bg && !c.panel && !c.fg && !look.font && !look.scale && look.radius == null && look.borders !== false && !look.glass && look.density !== 'compact'
    && !Object.keys(look.pictures ?? {}).length;
}

// ---- colour arithmetic, for the readability check ----

/** `#rgb` or `#rrggbb` as 0–255 channels; anything else is null. */
export function parseHex(c: string | undefined): [number, number, number] | null {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec((c ?? '').trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((x) => x + x).join('') : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
}

function luminance([r, g, b]: [number, number, number]): number {
  const f = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** WCAG contrast ratio of two hex colours, or null if either can't be read. */
export function contrast(a: string, b: string): number | null {
  const x = parseHex(a), y = parseHex(b);
  if (!x || !y) return null;
  const [l1, l2] = [luminance(x), luminance(y)].sort((p, q) => q - p);
  return (l1 + 0.05) / (l2 + 0.05);
}

/** Black or white, whichever reads better on `bg`. */
export function onColor(bg: string): string {
  return (contrast('#000000', bg) ?? 0) >= (contrast('#ffffff', bg) ?? 0) ? '#000000' : '#ffffff';
}

const VARS = ['--accent', '--accent-fg', '--accent-soft', '--bg', '--win', '--sunk', '--panel', '--line', '--line-2', '--fg', '--muted', '--faint', '--f-ui', '--r', '--glass-bg'];
const CLASSES = ['look-glass', 'look-noborders', 'look-compact'];

/**
 * Draws `look` over the theme on `el` (the page root). Returns whether text colour was
 * changed so it stays readable on the cards.
 */
export function applyLook(look: Look | null | undefined, el: HTMLElement = document.documentElement): { adjusted: boolean } {
  for (const v of VARS) el.style.removeProperty(v);
  el.classList.remove(...CLASSES);
  el.style.removeProperty('zoom');
  if (!look || isEmpty(look)) return { adjusted: false };

  const css = getComputedStyle(el);
  const base = (v: string) => css.getPropertyValue(v).trim();
  const set = (v: string, value: string) => el.style.setProperty(v, value);
  const c = look.colors ?? {};
  let adjusted = false;

  if (c.accent || c.bg || c.panel || c.fg) {
    const panel = c.panel ?? base('--panel');
    const bg = c.bg ?? base('--win');
    let fg = c.fg ?? base('--fg');
    // Text must stay readable on the cards: below 4.5:1 it becomes black or white.
    const ratio = contrast(fg, panel);
    if (ratio != null && ratio < 4.5) {
      fg = onColor(panel);
      adjusted = true;
    }
    if (c.bg) {
      set('--win', bg);
      set('--bg', bg);
    }
    if (c.bg || c.panel) set('--sunk', `color-mix(in oklab, ${panel} 45%, ${bg})`);
    if (c.panel) set('--panel', panel);
    if (c.fg || c.panel || adjusted) {
      set('--fg', fg);
      set('--muted', `color-mix(in oklab, ${fg} 62%, ${panel})`);
      set('--faint', `color-mix(in oklab, ${fg} 40%, ${panel})`);
      set('--line', `color-mix(in oklab, ${fg} 12%, ${panel})`);
      set('--line-2', `color-mix(in oklab, ${fg} 20%, ${panel})`);
    }
    if (c.accent) {
      set('--accent', c.accent);
      set('--accent-fg', onColor(c.accent));
      set('--accent-soft', `color-mix(in oklab, ${c.accent} 16%, ${panel})`);
    }
  }
  if (look.font && FONTS[look.font]) set('--f-ui', FONTS[look.font]);
  if (look.scale && look.scale !== 100) el.style.setProperty('zoom', String(clamp(look.scale, LIMITS.scale) / 100));
  if (look.radius != null) set('--r', `${clamp(look.radius, LIMITS.radius)}px`);
  if (look.borders === false) el.classList.add('look-noborders');
  if (look.glass) {
    el.classList.add('look-glass');
    set('--glass-bg', `color-mix(in srgb, var(--panel) ${100 - clamp(look.glass, LIMITS.glass)}%, transparent)`);
  }
  if (look.density === 'compact') el.classList.add('look-compact');
  return { adjusted };
}

const clamp = (v: number, [lo, hi]: readonly [number, number]) => Math.min(hi, Math.max(lo, v));
