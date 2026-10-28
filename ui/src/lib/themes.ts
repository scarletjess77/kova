import type { Settings } from './api';
import { t } from './i18n.svelte';

export type ThemeId = Settings['theme'];

/** A colour theme; the colours themselves live in app.css under `[data-theme=…]`. */
export interface ThemeInfo {
  id: Exclude<ThemeId, 'system'>;
  /** Monochrome themes have translated names; palettes keep their own. */
  label: () => string;
  light: boolean;
}

export const THEMES: ThemeInfo[] = [
  { id: 'dark', label: () => t('Dark'), light: false },
  { id: 'light', label: () => t('Light'), light: true },
  { id: 'oled', label: () => t('OLED black'), light: false },
  { id: 'dim', label: () => t('Dim'), light: false },
  { id: 'warm', label: () => t('Warm paper'), light: true },
  { id: 'contrast', label: () => t('High contrast'), light: false },
  { id: 'night', label: () => t('Night sky'), light: false },
  { id: 'nord', label: () => 'Nord', light: false },
  { id: 'catppuccin', label: () => 'Catppuccin', light: false },
  { id: 'dracula', label: () => 'Dracula', light: false },
  { id: 'gruvbox', label: () => 'Gruvbox', light: false },
  { id: 'solarized', label: () => 'Solarized', light: true },
];

/** The theme to draw: "system" follows the system's light or dark setting. */
export function resolveTheme(theme: ThemeId | undefined): ThemeInfo {
  const id = theme === 'system' ? (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : theme;
  return THEMES.find((x) => x.id === id) ?? THEMES[0];
}
