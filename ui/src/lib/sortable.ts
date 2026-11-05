// Reordering by dragging a handle, with pointer events so a mouse and a finger both work
// (the Android webview has no HTML drag and drop). Items are the direct children with
// `data-sort-id`; the handle is any element inside one with `data-handle`. While
// dragging, the item under the pointer is marked `drop-target`; letting go reports the
// move. Arrow keys on a focused handle move it one place.

export interface SortOptions {
  enabled: boolean;
  onmove: (from: number, to: number) => void;
}

export function sortable(node: HTMLElement, options: SortOptions) {
  let opts = options;
  let from = -1;
  let over = -1;
  let dragged: HTMLElement | null = null;

  const items = () => [...node.children].filter((c): c is HTMLElement => c instanceof HTMLElement && c.dataset.sortId != null);

  function nearest(x: number, y: number): number {
    let best = over;
    let dist = Infinity;
    items().forEach((el, i) => {
      const r = el.getBoundingClientRect();
      // Distance to the box, not its middle: big cards and small ones are judged alike.
      const dx = Math.max(r.left - x, 0, x - r.right);
      const dy = Math.max(r.top - y, 0, y - r.bottom);
      const d = Math.hypot(dx, dy);
      if (d < dist) { dist = d; best = i; }
    });
    return best;
  }

  function mark() {
    items().forEach((el, i) => el.classList.toggle('drop-target', i === over && i !== from));
  }

  function onMove(e: PointerEvent) {
    over = nearest(e.clientX, e.clientY);
    mark();
  }

  function onUp() {
    const handle = dragged?.querySelector<HTMLElement>('[data-handle]');
    handle?.removeEventListener('pointermove', onMove);
    dragged?.classList.remove('dragging');
    items().forEach((el) => el.classList.remove('drop-target'));
    const [a, b] = [from, over];
    dragged = null;
    from = over = -1;
    if (a >= 0 && b >= 0 && a !== b) opts.onmove(a, b);
  }

  function onDown(e: PointerEvent) {
    if (!opts.enabled || e.button > 0) return;
    const handle = (e.target as HTMLElement).closest<HTMLElement>('[data-handle]');
    const item = handle?.closest<HTMLElement>('[data-sort-id]');
    if (!handle || !item || item.parentElement !== node) return;
    e.preventDefault();
    dragged = item;
    from = over = items().indexOf(item);
    item.classList.add('dragging');
    handle.setPointerCapture(e.pointerId);
    handle.addEventListener('pointermove', onMove);
    handle.addEventListener('pointerup', onUp, { once: true });
    handle.addEventListener('pointercancel', onUp, { once: true });
  }

  function onKey(e: KeyboardEvent) {
    if (!opts.enabled) return;
    const handle = (e.target as HTMLElement).closest<HTMLElement>('[data-handle]');
    const item = handle?.closest<HTMLElement>('[data-sort-id]');
    if (!handle || !item || item.parentElement !== node) return;
    const step = e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowDown' || e.key === 'ArrowRight' ? 1 : 0;
    if (!step) return;
    e.preventDefault();
    const list = items();
    const i = list.indexOf(item);
    const j = i + step;
    if (j < 0 || j >= list.length) return;
    opts.onmove(i, j);
    // The list re-renders; keep the focus on the handle that moved.
    const id = item.dataset.sortId;
    requestAnimationFrame(() => node.querySelector<HTMLElement>(`[data-sort-id="${CSS.escape(id ?? '')}"] [data-handle]`)?.focus());
  }

  node.addEventListener('pointerdown', onDown);
  node.addEventListener('keydown', onKey);
  return {
    update(next: SortOptions) { opts = next; },
    destroy() {
      node.removeEventListener('pointerdown', onDown);
      node.removeEventListener('keydown', onKey);
    },
  };
}
