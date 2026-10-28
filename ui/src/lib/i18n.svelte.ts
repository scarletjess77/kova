// Interface language. The English text is the key: other languages map it to their
// translation, and anything without one shows in English. `npm run check` lists texts
// that are missing a translation.

import { fa } from './lang/fa';
import { ru } from './lang/ru';
import { zh } from './lang/zh';

export type Lang = 'en' | 'ru' | 'fa' | 'zh';
/** A translation, or for counted texts (see `tn`) the forms for one, few and many. */
export type Dict = Record<string, string | readonly [string, string, string]>;
type Params = Record<string, string | number>;

const DICTS: Record<Exclude<Lang, 'en'>, Dict> = { ru, fa, zh };
const LANGS: Lang[] = ['en', 'ru', 'fa', 'zh'];

class I18n {
  lang = $state<Lang>('en');
}
export const i18n = new I18n();

/** The language to show for the `language` setting ("system" or a language code). */
export function resolveLang(setting: string | undefined, system: string): Lang {
  if (LANGS.includes(setting as Lang)) return setting as Lang;
  const sys = system.toLowerCase();
  return LANGS.find((l) => sys.startsWith(l)) ?? 'en';
}

/** Persian reads right to left. */
export const isRtl = (lang: Lang) => lang === 'fa';

const fill = (s: string, p?: Params) => (p ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in p ? String(p[k]) : m)) : s);

export function t(en: string, params?: Params): string {
  const tr = i18n.lang === 'en' ? undefined : DICTS[i18n.lang][en];
  return fill(typeof tr === 'string' ? tr : en, params);
}

/** Like `t`, for an English text that needs different translations in different
 *  places; the translation is keyed `context|text`. */
export function tx(context: string, en: string, params?: Params): string {
  const tr = i18n.lang === 'en' ? undefined : DICTS[i18n.lang][`${context}|${en}`];
  return fill(typeof tr === 'string' ? tr : en, params);
}

const FORM: Partial<Record<Intl.LDMLPluralRule, number>> = { one: 0, few: 1, many: 2 };

/** A text with a count, given as the English singular and plural; `{n}` is the count.
 *  The translation is keyed by the plural and lists the language's own forms. */
export function tn(n: number, one: string, other: string, params?: Params): string {
  const p = { n, ...params };
  const tr = i18n.lang === 'en' ? undefined : DICTS[i18n.lang][other];
  if (Array.isArray(tr)) return fill(tr[FORM[new Intl.PluralRules(i18n.lang).select(n)] ?? 1], p);
  if (typeof tr === 'string') return fill(tr, p);
  return fill(n === 1 ? one : other, p);
}
