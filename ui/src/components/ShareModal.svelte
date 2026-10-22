<script lang="ts">
  import qrcode from 'qrcode-generator';
  import { api, type Profile } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { profile, onclose }: { profile: Profile; onclose: () => void } = $props();

  // What another device needs to add the same profile: the subscription address, or the
  // share links themselves. A config file (Clash, sing-box) is shared as text only.
  let text = $state<string | null>(null);
  void (async () => {
    if (profile.source.kind === 'remote') text = profile.source.url;
    else text = ((await app.run(() => api.getProfileContent(profile.id))) ?? '').trim();
  })();

  // A QR code holds about 2 KB comfortably; longer text is copied instead.
  const QR_LIMIT = 2000;
  let svg = $derived.by(() => {
    if (!text || text.length > QR_LIMIT || profile.format === 'clash' || profile.format === 'sing-box') return null;
    const qr = qrcode(0, 'L');
    qr.addData(text, 'Byte');
    qr.make();
    return qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true });
  });
</script>

<Modal title={t('Share {name}', { name: profile.name })} {onclose}>
  <p class="warn">{profile.source.kind === 'remote'
    ? t('Anyone with this link can use the subscription and its traffic. Share it only with your own devices or people you trust.')
    : t('Anyone with these links can use the servers. Share them only with your own devices or people you trust.')}</p>
  {#if text === null}
    <p class="muted">{t('Loading…')}</p>
  {:else}
    {#if svg}
      <!-- Dark on light in either theme, so any camera can read it. -->
      <div class="qr" aria-label={t('QR code of the profile')}>{@html svg}</div>
      <p class="hint">{t('Scan it in Kova on another device: Add profile → QR code.')}</p>
    {:else}
      <p class="hint">{profile.format === 'clash' || profile.format === 'sing-box'
        ? t('This profile is a config file, too large for a QR code. Copy it and paste it into Add profile on the other device.')
        : t('Too long for a QR code. Copy it and paste it into Add profile on the other device.')}</p>
    {/if}
    <textarea class="input mono" rows="3" readonly value={text}></textarea>
  {/if}

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>{t('Close')}</button>
    <button class="btn primary" disabled={!text} onclick={() => app.run(() => navigator.clipboard.writeText(text ?? ''), t('Copied'))}><Icon name="copy" />{t('Copy')}</button>
  {/snippet}
</Modal>

<style>
  .warn { margin: 0 0 12px; font-size: 12.5px; color: var(--warn); }
  .qr { width: 220px; height: 220px; margin: 0 auto; padding: 10px; border-radius: 10px; background: #fff; }
  .qr :global(svg) { width: 100%; height: 100%; display: block; }
  .hint { font-size: 12.5px; color: var(--muted); text-align: center; margin: 10px 0; }
  textarea { width: 100%; font-size: 12px; }
  .muted { color: var(--muted); }
</style>
