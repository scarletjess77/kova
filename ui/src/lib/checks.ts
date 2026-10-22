import type { Check } from './api';
import { t } from './i18n.svelte';
import { app } from './state.svelte';

/** A check step in words: what was checked, and what the result means. */
export function describe(c: Check): { title: string; text: string } {
  const d = c.detail ?? '';
  const ok = c.state === 'ok';
  switch (c.id) {
    case 'profile':
      return { title: t('A profile'), text: ok ? t('A profile is chosen.') : t('No profile yet. Add your servers first.') };
    case 'cores':
      return { title: t('Cores'), text: ok ? t('Mihomo and Xray are installed.') : t('Missing: {list}. Install them in Settings.', { list: d }) };
    case 'internet':
      return { title: t('The network itself'), text: ok ? t('The internet works without Kova.') : t('The internet doesn’t work even without Kova, so the problem is the network: Wi-Fi, mobile data or a sign-in page.') };
    case 'running':
      return { title: t('Connection'), text: ok ? t('Kova is connected.') : d ? t('Kova is not connected: {why}', { why: d }) : t('Kova is not connected.') };
    case 'node':
      if (c.state === 'warn') return { title: t('Server'), text: t('Direct mode is on, so no server is used at all.') };
      return { title: t('Server'), text: ok ? t('The server answers: {detail}.', { detail: d }) : c.state === 'skipped' ? '' : t('The server {name} doesn’t answer. Choose another one.', { name: d }) };
    case 'dns':
      return { title: t('Name lookups'), text: ok ? t('Site names are found.') : t('Site names can’t be looked up. Check the DNS servers in Settings.') };
    case 'through':
      return { title: t('Through Kova'), text: ok ? t('Sites open through Kova.') : t('Sites don’t open through Kova: {why}', { why: d }) };
    case 'capture':
      return { title: t('Your apps'), text: ok ? t('Apps are sent through Kova.') : t('Apps aren’t sent through Kova. Turn on System proxy or TUN mode.') };
    case 'exit':
      if (c.state === 'warn' && !app.status.running) return { title: t('Your address'), text: t('Not connected: sites see your own address {ip}.', { ip: d }) };
      if (c.state === 'warn') return { title: t('Your address'), text: t('Sites see the same address {ip} with Kova and without it. Traffic isn’t going through a server.', { ip: d }) };
      return { title: t('Your address'), text: ok ? t('Sites see {ip}, not your own address.', { ip: d }) : t('Could not find out what sites see.') };
    case 'dns-encrypted':
      return { title: t('Name lookups'), text: ok ? t('Encrypted, so your network can’t read which sites you look up.') : t('Not all DNS servers are encrypted, so your network can read which sites you look up. Use DoH servers (https://…) in Settings.') };
    case 'ipv6':
      return { title: 'IPv6', text: ok ? t('IPv6 traffic doesn’t go around Kova.') : t('This computer has IPv6, and with IPv6 off in Kova that traffic goes around TUN. Turn on IPv6 in Settings.') };
    case 'all-apps':
      return { title: t('Every app'), text: ok ? t('Everything goes through Kova, browsers’ WebRTC included.') : t('Only apps that follow the system proxy go through Kova. Others, and browsers’ WebRTC, can reach sites directly. TUN mode covers everything.') };
  }
  return { title: c.id, text: d };
}

/** The button that fixes a step, if one does. */
export function fixLabel(fix: string | null): string | null {
  switch (fix) {
    case 'add-profile': return t('Add profile');
    case 'install-cores': case 'settings': case 'tun': return t('Open Settings');
    case 'connect': return t('Connect');
    case 'switch-node': return t('Choose a server');
    case 'rule-mode': return t('Use Rule mode');
    case 'system-proxy': return app.mobile ? null : t('Turn on System proxy');
    case 'ipv6': return t('Turn on IPv6');
  }
  return null;
}

export async function runFix(fix: string, close: () => void) {
  switch (fix) {
    case 'add-profile': close(); app.go('profiles'); app.importOpen = true; break;
    case 'install-cores': case 'settings': case 'tun': close(); app.go('settings'); break;
    case 'connect': if (!app.status.running) await app.toggleConnection(); break;
    case 'switch-node': close(); app.go('proxies'); break;
    case 'rule-mode': await app.saveSettings((s) => (s.core.mode = 'rule'), t('Rule mode')); break;
    case 'system-proxy': await app.saveSettings((s) => (s.system_proxy = true), t('System proxy on')); break;
    case 'ipv6': await app.saveSettings((s) => (s.core.ipv6 = true), t('IPv6 on')); break;
  }
}
