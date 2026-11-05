// The user's layout: which tabs and Dashboard cards show, in what order. Phones and
// desktops keep one each, since a phone has room for much less. Saved in Kova's settings.

/** Tabs the user can hide and move. Dashboard (first) and Settings (last) always stay. */
export const TABS = ['proxies', 'profiles', 'connections', 'rules', 'check', 'logs'] as const;
export type TabId = (typeof TABS)[number];

/**
 * Dashboard cards. `connect` (the power button and status) can move but never hides;
 * `more` is the "More details" divider: cards below it wait behind that button.
 */
export const CARDS = ['connect', 'exit', 'quality', 'traffic', 'quick', 'more', 'tiles'] as const;
export type CardId = (typeof CARDS)[number];

export const FIXED_CARDS: readonly CardId[] = ['connect', 'more'];

export interface Item<T extends string = string> {
  id: T;
  hidden?: boolean;
}

export interface Layout {
  tabs: Item<TabId>[];
  cards: Item<CardId>[];
}

export type Form = 'desktop' | 'phone';
export type Preset = 'standard' | 'simple' | 'everything';
export const PRESETS: Preset[] = ['standard', 'simple', 'everything'];

/** What `settings.layout` holds. */
export interface SavedLayouts {
  desktop?: Layout;
  phone?: Layout;
}

const items = <T extends string>(order: readonly T[], hidden: readonly T[] = []): Item<T>[] =>
  order.map((id) => (hidden.includes(id) ? { id, hidden: true } : { id }));

/** A preset's layout for phones or desktops. */
export function preset(name: Preset, form: Form): Layout {
  const phone = form === 'phone';
  switch (name) {
    case 'simple':
      return {
        tabs: items(TABS, ['connections', 'rules', 'check', 'logs']),
        cards: phone
          ? items(['connect', 'quick', 'more', 'exit', 'quality', 'traffic', 'tiles'], ['quality', 'traffic', 'tiles'])
          : items(['connect', 'exit', 'quick', 'quality', 'traffic', 'tiles', 'more'], ['quality', 'traffic', 'tiles']),
      };
    case 'everything':
      return {
        tabs: items(TABS),
        cards: phone
          ? items(['connect', 'quick', 'exit', 'quality', 'traffic', 'tiles', 'more'])
          : items(['connect', 'exit', 'quality', 'traffic', 'quick', 'tiles', 'more']),
      };
    default:
      return {
        tabs: items(TABS),
        // Phones: the essentials first, the rest a tap away, as Kova always had it.
        cards: phone
          ? items(['connect', 'quick', 'more', 'exit', 'quality', 'traffic', 'tiles'])
          : items(['connect', 'exit', 'quality', 'traffic', 'quick', 'tiles', 'more']),
      };
  }
}

/** Keeps the known items of `saved` in its order and adds any it lacks (new in this version) where the default has them. */
function merge<T extends string>(saved: Item<T>[] | undefined, def: Item<T>[], fixed: readonly T[] = []): Item<T>[] {
  const known = new Set(def.map((i) => i.id));
  const out: Item<T>[] = [];
  const seen = new Set<T>();
  for (const i of saved ?? []) {
    if (!known.has(i.id) || seen.has(i.id)) continue;
    seen.add(i.id);
    out.push(fixed.includes(i.id) ? { id: i.id } : { id: i.id, ...(i.hidden ? { hidden: true } : {}) });
  }
  def.forEach((d, at) => {
    if (seen.has(d.id)) return;
    // Right after the item the default puts before it, or first.
    const before = def.slice(0, at).map((x) => x.id).reverse().find((id) => seen.has(id));
    const pos = before ? out.findIndex((x) => x.id === before) + 1 : 0;
    out.splice(pos, 0, { ...d });
    seen.add(d.id);
  });
  return out;
}

/** The layout to use: the saved one, completed from the standard one. */
export function resolve(saved: Layout | undefined, form: Form): Layout {
  const def = preset('standard', form);
  return { tabs: merge(saved?.tabs, def.tabs), cards: merge(saved?.cards, def.cards, FIXED_CARDS) };
}

/** Which preset `layout` is exactly, if any. */
export function presetOf(layout: Layout, form: Form): Preset | null {
  const same = (a: Item[], b: Item[]) => a.length === b.length && a.every((x, i) => x.id === b[i].id && !!x.hidden === !!b[i].hidden);
  return PRESETS.find((p) => {
    const l = preset(p, form);
    return same(layout.tabs, l.tabs) && same(layout.cards, l.cards);
  }) ?? null;
}

/** Moves the item at `from` to `to` in a copy of `list`. */
export function move<T>(list: T[], from: number, to: number): T[] {
  const out = [...list];
  const [x] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(out.length, to)), 0, x);
  return out;
}
