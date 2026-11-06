<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';

  // The bar lives in a shadow root on <body>, outside Kova's own tree: CSS being tried
  // can't reach inside it, and the host's inline !important rules beat anything aimed
  // at the host itself (`body * { display: none !important }` included). Should it be
  // hidden anyway, the countdown still undoes the CSS.
  const HOST = 'all: initial !important; display: block !important; visibility: visible !important; opacity: 1 !important; position: fixed !important; z-index: 2147483647 !important; left: 50% !important; bottom: 24px !important; transform: translateX(-50%) !important; pointer-events: auto !important;';
  const CSS = `
    .bar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 10px 12px; border-radius: 12px; background: #17171a; color: #ededef;
      border: 1px solid #3a3a40; box-shadow: 0 18px 50px rgb(0 0 0 / .5); font: 13px/1.4 system-ui, 'Segoe UI', sans-serif; max-width: calc(100vw - 32px); }
    button { font: 500 13px system-ui, 'Segoe UI', sans-serif; padding: 6px 12px; border-radius: 8px; border: 1px solid #3a3a40; background: #232327; color: #ededef; cursor: pointer; }
    button.keep { background: #f2f2f4; color: #111113; border-color: #f2f2f4; }`;

  let host: HTMLDivElement | null = null;
  let text: HTMLSpanElement | null = null;
  let keep: HTMLButtonElement | null = null;
  let undo: HTMLButtonElement | null = null;

  $effect(() => {
    const trial = app.cssTrial;
    if (!trial) {
      host?.remove();
      host = null;
      return;
    }
    if (!host) {
      host = document.createElement('div');
      host.setAttribute('style', HOST);
      host.setAttribute('role', 'alertdialog');
      const root = host.attachShadow({ mode: 'open' });
      const style = document.createElement('style');
      style.textContent = CSS;
      const bar = document.createElement('div');
      bar.className = 'bar';
      text = document.createElement('span');
      keep = document.createElement('button');
      keep.className = 'keep';
      keep.onclick = () => app.keepCss();
      undo = document.createElement('button');
      undo.onclick = () => app.undoCss();
      bar.append(text, keep, undo);
      root.append(style, bar);
      document.body.appendChild(host);
    }
    host.setAttribute('aria-label', t('Keep the custom CSS?'));
    text!.textContent = t('Keep the custom CSS? It is undone in {n} s.', { n: trial.left });
    keep!.textContent = t('Keep');
    undo!.textContent = t('Undo');
  });
</script>
