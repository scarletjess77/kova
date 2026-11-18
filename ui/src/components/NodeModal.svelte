<script lang="ts">
  import { api, errText, type NodeDetails } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { app } from '../lib/state.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  let { name, onclose }: { name: string; onclose: () => void } = $props();

  let details = $state<NodeDetails | null>(null);
  let error = $state<string | null>(null);
  // Rows whose secret is showing.
  let shown = $state<number[]>([]);

  void (async () => {
    try {
      details = await api.nodeDetails(app.settings?.active_profile ?? '', name);
    } catch (e) {
      error = errText(e);
    }
  })();

  // The engine sends English labels; they can't be extracted from data, so each is
  // spelled out here.
  let LABEL = $derived<Record<string, string>>({
    'Name': t('Name'), 'Protocol': t('Protocol'), 'Server': t('Server'), 'Port': t('Port'), 'Transport': t('Transport'), 'Path': t('Path'),
    'Host header': t('Host header'), 'Service name': t('Service name'), 'Mode': t('Mode'), 'Extra settings': t('Extra settings'),
    'Security': t('Security'), 'Server name (SNI)': t('Server name (SNI)'), 'ALPN': t('ALPN'), 'Fingerprint': t('Fingerprint'),
    'Allow insecure': t('Allow insecure'), 'Certificate pin': t('Certificate pin'), 'Encrypted Client Hello': t('Encrypted Client Hello'),
    'Reality public key': t('Reality public key'), 'Reality short ID': t('Reality short ID'), 'Reality spider path': t('Reality spider path'),
    'UDP': t('UDP'), 'Multiplexing': t('Multiplexing'), 'UUID': t('UUID'), 'Flow': t('Flow'), 'Encryption': t('Encryption'),
    'Alter ID': t('Alter ID'), 'Cipher': t('Cipher'), 'Method': t('Method'), 'Username': t('Username'), 'Password': t('Password'),
    'Padding': t('Padding'), 'Plugin': t('Plugin'), 'Plugin option': t('Plugin option'), 'Obfuscation': t('Obfuscation'),
    'Obfuscation password': t('Obfuscation password'), 'Port hopping': t('Port hopping'), 'Upload limit': t('Upload limit'),
    'Download limit': t('Download limit'), 'Congestion control': t('Congestion control'), 'UDP relay mode': t('UDP relay mode'),
    'Private key': t('Private key'), 'Public key': t('Public key'), 'Pre-shared key': t('Pre-shared key'), 'Addresses': t('Addresses'),
    'MTU': t('MTU'), 'Reserved bytes': t('Reserved bytes'), 'Type': t('Type'), 'Share link': t('Share link'),
  });
  const label = (l: string) => LABEL[l] ?? l;
  // The engine words the multiplexing value in English; protocol names stay as they are.
  const mux = (v: string): string => {
    const m = v.match(/^Xray Mux, (\d+) connections$/);
    if (m) return t('Xray Mux, {n} connections', { n: m[1] });
    return v.endsWith(', padding') ? mux(v.slice(0, -9)) + t(', padding') : v;
  };
  const plain = (v: string, l = '') => (v === 'Yes' ? t('Yes') : v === 'No' ? t('No') : l === 'Multiplexing' ? mux(v) : v);

  let share = $derived(details?.fields.find((f) => f.label === 'Share link')?.value ?? null);
  const flip = (i: number) => (shown = shown.includes(i) ? shown.filter((x) => x !== i) : [...shown, i]);
  const copy = (v: string) => app.run(() => navigator.clipboard.writeText(v), t('Copied'));
</script>

<Modal title={name} {onclose}>
  {#if error}
    <p class="bad">{error}</p>
  {:else if !details}
    <p class="muted">{t('Loading…')}</p>
  {:else}
    <p class="hint">{t('Secrets are hidden until you press Show. Anyone who sees them can use this server.')}</p>
    <dl>
      {#each details.fields as f, i (i)}
        {@const hide = f.secret && !shown.includes(i)}
        <div class="row">
          <dt>{label(f.label)}</dt>
          <dd class="mono selectable" class:masked={hide}>{hide ? '••••••••' : plain(f.value, f.label)}</dd>
          <div class="ops">
            {#if f.secret}
              <button class="x" onclick={() => flip(i)} aria-pressed={!hide} title={hide ? t('Show') : t('Hide')} aria-label={hide ? t('Show') : t('Hide')}><Icon name={hide ? 'eye' : 'eye-off'} /></button>
            {/if}
            <button class="x" onclick={() => copy(f.value)} title={t('Copy')} aria-label={t('Copy')}><Icon name="copy" /></button>
          </div>
        </div>
      {/each}
    </dl>
    {#if details.raw}
      <details>
        <summary>{t('Raw configuration')}</summary>
        <textarea class="input mono raw" readonly rows="10" value={details.raw}></textarea>
      </details>
    {/if}
  {/if}
  {#snippet footer()}
    {#if share}<button class="btn" onclick={() => copy(share!)}><Icon name="link" />{t('Copy share link')}</button>{/if}
    <button class="btn" onclick={onclose}>{t('Close')}</button>
  {/snippet}
</Modal>

<style>
  .hint { margin: 0; }
  .bad { color: var(--bad); margin: 0; }
  dl { margin: 0; display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: var(--r); }
  .row { display: grid; grid-template-columns: minmax(110px, 170px) minmax(0, 1fr) auto; gap: 4px 12px; align-items: center; padding: 6px 6px 6px 12px; }
  .row + .row { border-top: 1px solid var(--line); }
  dt { color: var(--muted); font-size: 12px; }
  dd { margin: 0; font-size: 12px; overflow-wrap: anywhere; min-width: 0; }
  dd.masked { color: var(--faint); letter-spacing: 1px; }
  .ops { display: flex; gap: 2px; }
  summary { cursor: pointer; font-size: 13px; color: var(--muted); }
  .raw { margin-top: 8px; width: 100%; resize: vertical; font-size: 12px; white-space: pre; }
  @media (max-width: 480px) {
    .row { grid-template-columns: minmax(0, 1fr) auto; }
    dt { grid-column: 1 / -1; }
  }
</style>
