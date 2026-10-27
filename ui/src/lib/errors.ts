import { t } from './i18n.svelte';

// The engine reports errors in English, partly in the words of the system or the core.
// These patterns turn the known ones into plain words in the interface's language; any
// other message is shown as it is.
type Rule = [RegExp, (m: RegExpMatchArray) => string];

const RULES: Rule[] = [
  // Kova's own messages.
  [/^the server took too long to answer$/i, () => t('The server took too long to answer')],
  [/^could not connect to the server\. Check the address and your connection$/i, () => t('Could not connect to the server. Check the address and your connection')],
  [/^the server sent something that is not a subscription$/i, () => t('The server sent something that is not a subscription')],
  [/^the server answered (\d{3})/i, (m) => serverStatus(m[1])],
  [/^GitHub answered (\d{3})/i, (m) => t('GitHub answered {status}. Try again later', { status: m[1] })],
  [/^the content has no proxies we can read$/i, () => t('No servers found in it. Check that it is a subscription or share links')],
  [/^invalid Clash config: (.*)$/i, (m) => t('The Clash profile is invalid: {detail}', { detail: m[1] })],
  [/^the config is invalid: (.*)$/i, (m) => t('The config is invalid: {detail}', { detail: m[1] })],
  [/^the profile is not a valid YAML mapping/i, () => t('The profile is not valid YAML')],
  [/^unsupported link type `(.+)`$/i, (m) => t('Links of type {kind} are not supported', { kind: m[1] })],
  [/^not a share link$/i, () => t('This is not a share link')],
  [/^invalid (\S+) link: (.*)$/i, (m) => t('The {kind} link is invalid: {detail}', { kind: m[1], detail: m[2] })],
  [/^Add a profile first$/, () => t('Add a profile first')],
  [/^The active profile was removed\. Choose another one$/, () => t('The active profile was removed. Choose another one')],
  [/^profile not found$/i, () => t('The profile was not found')],
  [/^the profile was removed during the update$/i, () => t('The profile was removed during the update')],
  [/^Port (\d+) is already used by another program/, (m) => t('Port {port} is already used by another program. Close it or choose another port in Settings', { port: m[1] })],
  [/^Xray profiles also use the port above the proxy port/, () => t('Xray profiles also use the port above the proxy port; choose a proxy port below 65535')],
  [/^Mihomo did not start in time\.?\s*(.*)$/i, (m) => (m[1] ? t('Mihomo did not start in time: {detail}', { detail: m[1] }) : t('Mihomo did not start in time'))],
  [/^could not start (\S+): (.*)$/i, (m) => t('Could not start {name}: {detail}', { name: m[1], detail: friendly(m[2]) })],
  [/^(\S+) is not installed$/i, (m) => t('{name} is not installed. Install it in Settings, Cores', { name: m[1] })],
  [/checksum (mismatch|check)/i, () => t('A download failed its checksum check, so it was not installed. Try again')],
  [/has no published checksum|has no checksum for/i, () => t('The release has no checksum to verify, so Kova did not install it')],
  [/^no (\S+) build for this system/i, (m) => t('{name} has no build for this system', { name: m[1] })],
  [/^the Kova service is only available on Windows/i, () => t('The Kova service is only available on Windows for now')],
  [/^the Kova service did not answer in time$/i, () => t('The Kova service did not answer in time')],
  [/^the Kova service is busy$/i, () => t('The Kova service is busy. Try again in a moment')],
  [/^the Kova service is not running/i, () => t('The Kova service is not running. Repair it in Settings, TUN mode')],
  [/^TUN mode needs the Kova service/i, () => t('TUN mode needs the Kova service. Install it in Settings, TUN mode')],
  [/^TUN mode needs the Mihomo core/i, () => t('TUN mode needs the Mihomo core; this profile runs on Xray only')],
  [/^Kova needs permission to set up a VPN/i, () => t('Kova needs permission to set up a VPN to connect')],
  [/^Android did not allow the VPN/i, () => t('Android did not allow the VPN. If another app is set as always-on VPN, turn that off first')],
  [/^None of the apps chosen for the VPN is installed/i, () => t('None of the apps chosen for the VPN is installed. Choose apps in Settings')],
  [/^The VPN was turned off by Android or another app$/i, () => t('The VPN was turned off by Android or another app')],
  [/^Enter an address starting with https:\/\/$/, () => t('Enter an address starting with https://')],
  [/^That file is larger than 8 MB/, () => t('That file is larger than 8 MB; it is unlikely to be a config')],
  [/^Check for updates first$/, () => t('Check for updates first')],
  [/^Allow Kova to install apps, then press Download update again$/, () => t('Allow Kova to install apps, then press Download update again')],
  [/^Connected without the routing data/, () => t('Connected without the routing data, which couldn’t be downloaded directly; getting it through the server')],
  [/^Routing data downloaded through the server/, () => t('Routing data downloaded through the server; every rule works now')],
  [/^Anti-DPI needs the Xray core/, () => t('Anti-DPI needs the Xray core. Install it in Settings, Cores')],
  // What the system and the cores say, in plain words.
  [/i\/o timeout|timed out|deadline exceeded|operation timed out/i, () => t('No answer in time. The server may be down or blocked')],
  [/connection refused|actively refused/i, () => t('The connection was refused. The server may be down or the port wrong')],
  [/connection reset|reset by peer|forcibly closed/i, () => t('The connection was cut, which is typical of blocking')],
  [/no such host|name not resolved|failed to lookup|dns/i, () => t('The name could not be resolved. Check the address and your connection')],
  [/certificate|x509|tls|handshake/i, () => t('The secure connection failed. The server’s certificate or the connection was tampered with')],
  [/network is unreachable|no route to host|host is unreachable/i, () => t('The network can’t be reached. Check your connection')],
  [/access is denied|permission denied|operation not permitted/i, () => t('Access was denied. Kova may need more rights for this')],
  [/address already in use|only one usage of each socket address/i, () => t('A port Kova needs is already used by another program')],
];

function serverStatus(code: string): string {
  if (code === '401' || code === '403') return t('The server refused access ({status}). The subscription may have expired', { status: code });
  if (code === '404') return t('Nothing found at that address (404). Check the subscription link');
  if (code.startsWith('5')) return t('The server has a problem ({status}). Try again later', { status: code });
  return t('The server answered {status}', { status: code });
}

/** An engine error in plain words, in the interface's language. */
export function friendly(message: string): string {
  const msg = message.trim();
  for (const [re, f] of RULES) {
    const m = msg.match(re);
    if (m) return f(m);
  }
  return msg.charAt(0).toUpperCase() + msg.slice(1);
}
