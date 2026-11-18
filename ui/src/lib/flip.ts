// Slides the items of a list to their new places when the list changes, so a
// tab or card that is hidden or moved does not make everything below it jump at once.
// Items are the elements inside it with `data-sort-id`. Pass anything that changes with the list (its
// ids and places) as the parameter: the action compares places on every change of it.
// No animation while an item is being dragged, or when the system asks for less motion.

const MS = 160;

export function flip(node: HTMLElement, _key?: unknown) {
  type Box = { x: number; y: number };
  let before = new Map<string, Box>();
  const running = new Map<string, Animation>();

  const items = () => [...node.querySelectorAll<HTMLElement>('[data-sort-id]')];
  // Places inside the list, so scrolling does not count as a move.
  function measure(): Map<string, Box> {
    const base = node.getBoundingClientRect();
    const out = new Map<string, Box>();
    for (const el of items()) {
      const r = el.getBoundingClientRect();
      out.set(el.dataset.sortId!, { x: r.left - base.left, y: r.top - base.top });
    }
    return out;
  }

  function settle() {
    // Where an item still on its way is on screen counts as its start when a new move
    // interrupts it; the others start where they last stood.
    const seen = measure();
    const out = new Map<string, Box>();
    for (const id of running.keys()) { const s = seen.get(id); if (s) out.set(id, s); }
    for (const a of running.values()) a.cancel();
    running.clear();
    return out;
  }

  const stillBefore = () => { before = measure(); };
  // The window or a card changing size moves things without a change of the list.
  const ro = new ResizeObserver(() => { if (running.size === 0) stillBefore(); });
  ro.observe(node);
  stillBefore();
  // Whatever changes the list starts with a click, a key or a drop; take the places just
  // before it, so content that loaded since the last change cannot look like a move.
  const fresh = () => { if (running.size === 0) stillBefore(); };
  const EVENTS = ['pointerdown', 'pointerup', 'click', 'keydown'] as const;
  for (const e of EVENTS) document.addEventListener(e, fresh, true);

  return {
    update() {
      const visual = settle();
      const after = measure();
      const calm = matchMedia('(prefers-reduced-motion: reduce)').matches || node.querySelector('.dragging');
      if (!calm) {
        for (const el of items()) {
          const id = el.dataset.sortId!;
          const from = visual.get(id) ?? before.get(id);
          const to = after.get(id);
          if (!from || !to) continue;
          const dx = from.x - to.x;
          const dy = from.y - to.y;
          if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
          const a = el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: MS, easing: 'ease-out' });
          running.set(id, a);
          a.onfinish = a.oncancel = () => { if (running.get(id) === a) running.delete(id); };
        }
      }
      before = after;
    },
    destroy() {
      ro.disconnect();
      for (const e of EVENTS) document.removeEventListener(e, fresh, true);
      for (const a of running.values()) a.cancel();
    },
  };
}
