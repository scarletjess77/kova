// Android TV: the remote's arrow keys move focus the way TV apps do; the WebView alone
// would only scroll. OK clicks the focused control, and Back goes back as on a phone.
//
// - A press moves to the nearest control in that direction, preferring ones in line with
//   the current one (the rule Android's own focus search uses), so focus never jumps
//   diagonally across the screen.
// - Left from the page's edge goes to the side menu, onto the current page; right from
//   the menu returns to where focus was on the page. Opening a page moves focus into it.
// - The focused control is kept in the middle of the screen, and where nothing is left
//   below, Down scrolls the page so text-only parts can be read.

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

type Dir = 'up' | 'down' | 'left' | 'right';
const KEYS: Record<string, Dir> = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };

/** On screen and reachable: not hidden, not off to a side, not inside something hidden. */
function usable(el: HTMLElement): boolean {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0 || r.right <= 0 || r.left >= window.innerWidth) return false;
  if (el.closest('[aria-hidden="true"], [inert], [hidden]')) return false;
  return getComputedStyle(el).visibility !== 'hidden';
}

/** Where focus may go: inside an open dialog only its own controls count. */
function scope(): ParentNode {
  const dialogs = document.querySelectorAll('[role="dialog"]');
  return dialogs.length ? dialogs[dialogs.length - 1] : document;
}

const inDialog = () => scope() !== document;
const side = () => document.querySelector<HTMLElement>('.side');
const main = () => document.querySelector<HTMLElement>('.main');

function focusables(within: ParentNode = scope()): HTMLElement[] {
  return [...within.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(usable);
}

/**
 * The best control from `from` in `dir`; candidates must lie beyond the current edge.
 * Up and down go row by row: the nearest row first, then the control in it most in line
 * with the current one. Left and right stay in the row when they can, nearest first,
 * and otherwise weigh the distance across heavily, as Android's own focus search does.
 */
function next(from: HTMLElement, dir: Dir, within: ParentNode = scope()): HTMLElement | null {
  const a = from.getBoundingClientRect();
  const vertical = dir === 'up' || dir === 'down';
  const found: { el: HTMLElement; major: number; minor: number; overlap: number }[] = [];
  for (const el of focusables(within)) {
    if (el === from || el.contains(from) || from.contains(el)) continue;
    const b = el.getBoundingClientRect();
    // Beyond the current edge, with a little slack for touching rows.
    const ahead = dir === 'down' ? b.top >= a.bottom - 4 : dir === 'up' ? b.bottom <= a.top + 4 : dir === 'right' ? b.left >= a.right - 4 : b.right <= a.left + 4;
    if (!ahead) continue;
    const major = Math.max(0, dir === 'down' ? b.top - a.bottom : dir === 'up' ? a.top - b.bottom : dir === 'right' ? b.left - a.right : a.left - b.right);
    const overlap = vertical ? Math.min(a.right, b.right) - Math.max(a.left, b.left) : Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    const minor = vertical ? Math.abs((a.left + a.right) / 2 - (b.left + b.right) / 2) : Math.abs((a.top + a.bottom) / 2 - (b.top + b.bottom) / 2);
    found.push({ el, major, minor, overlap });
  }
  if (!found.length) return null;
  if (vertical) {
    const nearest = Math.min(...found.map((c) => c.major));
    const row = found.filter((c) => c.major <= nearest + 24);
    row.sort((x, y) => Math.max(0, y.overlap) - Math.max(0, x.overlap) || x.minor - y.minor);
    return row[0].el;
  }
  const score = (c: (typeof found)[number]) => (c.overlap > 0 ? c.major : 1e6 + 13 * c.major ** 2 + c.minor ** 2);
  found.sort((x, y) => score(x) - score(y));
  return found[0].el;
}

function focus(el: HTMLElement) {
  el.focus({ preventScroll: true });
  el.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'smooth' });
}

/** Where focus was on the page before going to the menu, to come back to. */
let lastInPage: HTMLElement | null = null;

/** Put focus somewhere sensible when nothing in reach has it: the main button, else the
 *  first control of the page or the open dialog. */
function focusStart(): boolean {
  const active = document.activeElement as HTMLElement | null;
  if (active && active !== document.body && scope().contains(active) && usable(active)) return true;
  const page = main();
  const start = (!inDialog() && document.querySelector<HTMLElement>('.power:not([disabled])')) || (inDialog() ? focusables()[0] : page && focusables(page)[0]) || focusables()[0];
  if (!start || !usable(start)) return false;
  focus(start);
  return true;
}

function intoPage(): boolean {
  const page = main();
  if (!page) return false;
  const back = lastInPage && page.contains(lastInPage) && usable(lastInPage) ? lastInPage : focusables(page)[0];
  if (!back) return false;
  focus(back);
  return true;
}

function move(dir: Dir) {
  const active = document.activeElement as HTMLElement | null;
  // Nothing focused yet, or focus left behind outside an open dialog: start there.
  if (!active || active === document.body || !scope().contains(active) || !usable(active)) {
    focusStart();
    return;
  }
  const menu = side();
  const inMenu = !!menu && menu.contains(active);
  if (!inDialog()) {
    if (inMenu && dir === 'right') {
      if (intoPage()) return;
    }
    if (!inMenu && dir === 'left') {
      const target = next(active, 'left', main() ?? document);
      if (!target && menu) {
        if (main()?.contains(active)) lastInPage = active;
        const current = menu.querySelector<HTMLElement>('[aria-current="page"]') ?? focusables(menu)[0];
        if (current) focus(current);
        return;
      }
    }
  }
  // Inside the menu, the page or a dialog: move within it first.
  const area = inDialog() ? scope() : inMenu ? menu! : document;
  const target = next(active, dir, area);
  if (target) {
    if (!inDialog() && main()?.contains(target)) lastInPage = target;
    focus(target);
    return;
  }
  // Nothing further down: show the rest of the page.
  const page = main();
  if (!inDialog() && page && (dir === 'down' || dir === 'up') && !inMenu) {
    page.scrollBy({ top: (dir === 'down' ? 1 : -1) * page.clientHeight * 0.6, behavior: 'smooth' });
  }
}

export function enableTvNavigation() {
  document.documentElement.classList.add('tv');
  window.addEventListener('keydown', (e) => {
    const dir = KEYS[e.key];
    if (!dir) return;
    const active = document.activeElement as HTMLElement | null;
    // Text fields keep their arrows for the cursor, except up and down out of one line.
    if (active?.matches('textarea') || (active?.matches('input') && (dir === 'left' || dir === 'right'))) return;
    e.preventDefault();
    move(dir);
  });
  // Opening a page from the menu takes focus into it.
  document.addEventListener('click', (e) => {
    const menu = side();
    if (inDialog() || !menu || !menu.contains(e.target as Node)) return;
    lastInPage = null;
    setTimeout(() => {
      const page = main();
      const first = page && focusables(page)[0];
      if (first) focus(first);
    }, 150);
  });
  // Start on the main control once the page is drawn, and inside each dialog that opens.
  let tries = 0;
  const first = setInterval(() => {
    if (focusStart() || ++tries > 20) clearInterval(first);
  }, 300);
  // Android hands the page focus a moment after it is drawn; start again then.
  window.addEventListener('focus', () => focusStart());
  new MutationObserver(() => {
    if (inDialog()) requestAnimationFrame(() => focusStart());
  }).observe(document.body, { childList: true, subtree: true });
}
