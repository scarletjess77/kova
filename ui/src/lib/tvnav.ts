// Android TV: the remote's arrow keys move focus to the nearest control in that
// direction, the way TV apps behave; the WebView alone would only scroll. OK clicks
// the focused control, and Back goes back as on a phone.

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

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

function focusables(): HTMLElement[] {
  return [...scope().querySelectorAll<HTMLElement>(FOCUSABLE)].filter(usable);
}

/** The control nearest to `from` in direction (dx, dy), favouring ones straight ahead. */
function next(from: DOMRect, dx: number, dy: number): HTMLElement | null {
  const cx = from.left + from.width / 2;
  const cy = from.top + from.height / 2;
  let best: HTMLElement | null = null;
  let bestScore = Infinity;
  for (const el of focusables()) {
    if (el === document.activeElement) continue;
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2 - cx;
    const y = r.top + r.height / 2 - cy;
    const along = x * dx + y * dy;
    if (along <= 1) continue;
    const across = Math.abs(x * dy - y * dx);
    const score = along + across * 2;
    if (score < bestScore) {
      bestScore = score;
      best = el;
    }
  }
  return best;
}

function focus(el: HTMLElement) {
  el.focus();
  el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

/** Put focus somewhere sensible when nothing in reach has it: the main button, else the
 *  first control of the page or the open dialog. */
function focusStart(): boolean {
  const active = document.activeElement as HTMLElement | null;
  if (active && active !== document.body && scope().contains(active) && usable(active)) return true;
  const inDialog = scope() !== document;
  const start = (!inDialog && document.querySelector<HTMLElement>('.power:not([disabled])')) || focusables()[0];
  if (!start || !usable(start)) return false;
  focus(start);
  return true;
}

const DIRS: Record<string, [number, number]> = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] };

export function enableTvNavigation() {
  document.documentElement.classList.add('tv');
  window.addEventListener('keydown', (e) => {
    const dir = DIRS[e.key];
    if (!dir) return;
    const active = document.activeElement as HTMLElement | null;
    // Text fields keep their arrows for the cursor, except up and down out of one line.
    if (active?.matches('textarea') || (active?.matches('input') && (e.key === 'ArrowLeft' || e.key === 'ArrowRight'))) return;
    e.preventDefault();
    // Nothing focused yet, or focus left behind outside an open dialog: start there.
    if (!active || active === document.body || !scope().contains(active) || !usable(active)) {
      focusStart();
      return;
    }
    const target = next(active.getBoundingClientRect(), dir[0], dir[1]);
    if (target) focus(target);
  });
  // Start on the main control once the page is drawn, and inside each dialog that opens.
  let tries = 0;
  const first = setInterval(() => {
    if (focusStart() || ++tries > 20) clearInterval(first);
  }, 300);
  // Android hands the page focus a moment after it is drawn; start again then.
  window.addEventListener('focus', () => focusStart());
  new MutationObserver(() => {
    if (scope() !== document) requestAnimationFrame(() => focusStart());
  }).observe(document.body, { childList: true, subtree: true });
}
