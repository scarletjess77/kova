<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import qrcode from 'qrcode-generator';
  import { TON_ADDRESS, TON_URI, TONKEEPER_URL } from '../lib/donate';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';

  const qr = qrcode(0, 'M');
  qr.addData(TON_URI);
  qr.make();
  const svg = qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true });

  let copied = $state(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(TON_ADDRESS);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      app.notify('Could not copy; select the address and copy it by hand');
    }
  }
</script>

<div class="donate">
  <!-- The QR code needs dark-on-light contrast in either theme. -->
  <div class="qr" aria-label="QR code with the TON address">{@html svg}</div>
  <div class="text">
    <p>Kova is free and has no ads or tracking. If it helps you, you can support its development with a donation in TON.</p>
    <div class="eyebrow">TON wallet</div>
    <div class="addr mono selectable">{TON_ADDRESS}</div>
    <div class="actions">
      <button class="btn primary" onclick={copy}><Icon name={copied ? 'check' : 'copy'} />{copied ? 'Copied' : 'Copy address'}</button>
      <button class="btn" onclick={() => app.run(() => openUrl(TONKEEPER_URL))}><Icon name="external" />Open in Tonkeeper</button>
    </div>
    <p class="hint">Or scan the code with any TON wallet. Thank you!</p>
  </div>
</div>

<style>
  .donate { display: flex; gap: 18px; align-items: flex-start; flex-wrap: wrap; }
  .qr { width: 148px; height: 148px; flex: none; background: #fff; border-radius: 10px; padding: 6px; }
  .qr :global(svg) { width: 100%; height: 100%; display: block; }
  .text { flex: 1; min-width: 240px; display: flex; flex-direction: column; gap: 8px; }
  .text p { margin: 0; color: var(--muted); font-size: 13px; }
  .addr { font-size: 12.5px; word-break: break-all; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--sunk); }
</style>
