// Pictures in the look: one per slot (behind every tab, behind one tab, the sidebar, the
// connect card). Kova keeps its own copy of each in its data folder; the interface
// shows them through blob URLs. Templates show where Kova's own parts sit on a
// picture of the recommended size.

import { invoke } from '@tauri-apps/api/core';

export type Fit = 'cover' | 'contain' | 'tile';
export type Pos = 'center' | 'top' | 'bottom' | 'left' | 'right';

export interface Picture {
  /** The kept copy's name in Kova's folder. */
  file: string;
  fit?: Fit;
  pos?: Pos;
  /** How much the window colour covers the picture, 0 to 90 percent; text stays readable. */
  dim?: number;
  /** Blur in px, 0 to 20; backgrounds only. */
  blur?: number;
}

export const TAB_VIEWS = ['dashboard', 'proxies', 'profiles', 'connections', 'rules', 'check', 'logs', 'settings'] as const;
export type SlotId = 'app' | 'app-phone' | 'sidebar' | 'banner' | `tab-${(typeof TAB_VIEWS)[number]}`;

/** Recommended size of each slot's picture. */
export function slotSize(slot: SlotId): [number, number] {
  if (slot === 'app-phone') return [1080, 2400];
  if (slot === 'sidebar') return [480, 1440];
  if (slot === 'banner') return [1600, 400];
  return [2560, 1440];
}

/** Whether the slot is a background behind the window, which may be blurred. */
export const isBackdrop = (slot: SlotId) => slot === 'app' || slot === 'app-phone' || slot.startsWith('tab-');

export const DEFAULT_DIM = 35;

/** CSS for an element showing `pic`: the picture under a veil of the window colour. */
export function pictureStyle(pic: Picture, url: string): string {
  const dim = Math.min(90, Math.max(0, pic.dim ?? DEFAULT_DIM));
  const veil = `color-mix(in srgb, var(--win) ${dim}%, transparent)`;
  const fit = pic.fit ?? 'cover';
  return [
    `background-image: linear-gradient(${veil}, ${veil}), url("${url}")`,
    `background-size: auto, ${fit === 'tile' ? 'auto' : fit}`,
    `background-repeat: no-repeat, ${fit === 'tile' ? 'repeat' : 'no-repeat'}`,
    `background-position: center, ${pic.pos ?? 'center'}`,
  ].join('; ');
}

// ---- kept copies ----

const TYPES: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp' };

/** Hands a chosen file to Kova to keep; returns the kept copy's name. */
export async function keepPicture(slot: SlotId, file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<string>('look_picture_save', bytes, { headers: { slot } });
}

export const removePicture = (slot: SlotId) => invoke<void>('look_picture_remove', { slot });

/** A blob URL for a kept copy. */
export async function loadPicture(name: string): Promise<string> {
  const buf = await invoke<ArrayBuffer>('look_picture', { name });
  return URL.createObjectURL(new Blob([buf], { type: TYPES[name.split('.').pop() ?? ''] ?? 'image/png' }));
}

// ---- templates ----

interface Zone { x: number; y: number; w: number; h: number; label: string }

/** Where Kova's own parts sit on a slot's picture, as fractions of its size. */
function zones(slot: SlotId, L: Record<string, string>): Zone[] {
  switch (slot) {
    case 'app-phone':
      return [
        { x: 0, y: 0, w: 1, h: 0.05, label: L.top },
        { x: 0.04, y: 0.08, w: 0.92, h: 0.84, label: L.cards },
      ];
    case 'sidebar':
      return [
        { x: 0.05, y: 0.03, w: 0.9, h: 0.36, label: L.tabs },
        { x: 0.05, y: 0.84, w: 0.9, h: 0.13, label: L.profile },
      ];
    case 'banner':
      return [
        { x: 0.03, y: 0.18, w: 0.15, h: 0.6, label: L.power },
        { x: 0.21, y: 0.1, w: 0.42, h: 0.5, label: L.status },
        { x: 0.21, y: 0.68, w: 0.55, h: 0.26, label: L.controls },
      ];
    default:
      return [
        { x: 0, y: 0, w: 1, h: 0.03, label: L.title },
        { x: 0, y: 0.03, w: 0.16, h: 0.95, label: L.sidebar },
        { x: 0.18, y: 0.07, w: 0.8, h: 0.87, label: L.cards },
        { x: 0, y: 0.98, w: 1, h: 0.02, label: '' },
      ];
  }
}

/** A PNG of the slot's recommended size with Kova's parts marked, for drawing on. */
export async function templatePng(slot: SlotId, title: string, labels: Record<string, string>): Promise<Uint8Array> {
  const [w, h] = slotSize(slot);
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  const g = c.getContext('2d')!;
  g.fillStyle = '#26262b';
  g.fillRect(0, 0, w, h);
  g.strokeStyle = 'rgba(255,255,255,0.06)';
  g.lineWidth = 1;
  for (let x = 0; x < w; x += 80) { g.beginPath(); g.moveTo(x + 0.5, 0); g.lineTo(x + 0.5, h); g.stroke(); }
  for (let y = 0; y < h; y += 80) { g.beginPath(); g.moveTo(0, y + 0.5); g.lineTo(w, y + 0.5); g.stroke(); }
  const unit = Math.max(16, Math.round(Math.min(w, h) / 30));
  for (const z of zones(slot, labels)) {
    const [x, y, zw, zh] = [z.x * w, z.y * h, z.w * w, z.h * h];
    g.fillStyle = 'rgba(255,255,255,0.10)';
    g.fillRect(x, y, zw, zh);
    g.setLineDash([unit / 2, unit / 3]);
    g.strokeStyle = 'rgba(255,255,255,0.55)';
    g.lineWidth = 2;
    g.strokeRect(x + 1, y + 1, zw - 2, zh - 2);
    g.setLineDash([]);
    if (z.label) {
      // Thin strips (the title bar) get a label that fits inside them.
      const size = Math.min(unit, zh * 0.55);
      g.fillStyle = 'rgba(255,255,255,0.85)';
      g.font = `500 ${size}px system-ui, sans-serif`;
      g.textBaseline = 'middle';
      g.fillText(z.label, x + unit * 0.6, y + Math.min(unit, zh / 2), Math.max(10, zw - unit));
      g.textBaseline = 'alphabetic';
    }
  }
  // The name and size, bottom right, clear of the marked parts.
  g.fillStyle = 'rgba(255,255,255,0.7)';
  g.font = `500 ${Math.round(unit * 0.7)}px system-ui, sans-serif`;
  g.textAlign = 'right';
  g.fillText(`Kova · ${title} · ${w} × ${h}`, w - unit * 0.6, h - unit * 0.6);
  const blob = await new Promise<Blob | null>((r) => c.toBlob(r, 'image/png'));
  return new Uint8Array(await blob!.arrayBuffer());
}

export const saveChosenFile = (path: string, bytes: Uint8Array) =>
  invoke<void>('save_chosen_file', bytes, { headers: { path: encodeURIComponent(path) } });
