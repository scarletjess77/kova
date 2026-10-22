// Android TV: the remote's arrow keys move focus to the nearest control in that
// direction, the way TV apps behave; the WebView alone would only scroll. OK clicks
// the focused control, and Back goes back as on a phone.

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function visible(el: HTMLElement): boolean {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden';
}

/** The control nearest to `from` in direction (dx, dy), favouring ones straight ahead. */
function next(from: DOMRect, dx: number, dy: number): HTMLElement | null {
  const cx = from.left + from.width / 2;
  const cy = from.top + from.height / 2;
  let best: HTMLElement | null = null;
  let bestScore = Infinity;
  // Inside an open dialog, only its own controls count.
  const scope = document.querySelector('[role="dialog"]') ?? document;
  for (const el of scope.querySelectorAll<HTMLElement>(FOCUSABLE)) {
    if (el === document.activeElement || !visible(el)) continue;
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

const DIRS: Record<string, [number, number]> = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] };

export function enableTvNavigation() {
  document.documentElement.classList.add('tv');
  window.addEventListener('keydown', (e) => {
    const dir = DIRS[e.key];
    if (!dir) return;
    const active = document.activeElement as HTMLElement | null;
    // Text fields keep their arrows for the cursor, except up and down out of one line.
    if (active?.matches('textarea') || (active?.matches('input') && (e.key === 'ArrowLeft' || e.key === 'ArrowRight'))) return;
    const from = active && active !== document.body ? active.getBoundingClientRect() : new DOMRect(0, 0, 0, 0);
    const target = next(from, dir[0], dir[1]);
    if (target) {
      e.preventDefault();
      target.focus();
      target.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }
  });
  // Start on the main control so the first press does something visible.
  setTimeout(() => document.querySelector<HTMLElement>('.power')?.focus(), 500);
}
