// "Copy report": what support needs to see in one paste, with nothing that would let
// someone else use the subscription. Left in English, for whoever reads it.

import { getVersion } from '@tauri-apps/api/app';
import { api, type Check } from './api';
import { app } from './state.svelte';

/** Takes out what identifies the subscription or the device: links (which carry the
 *  access key), UUIDs used as passwords, long tokens and Kova's own secrets. */
export function redact(text: string, secrets: string[] = []): string {
  let out = text;
  for (const s of secrets) if (s && s.length >= 6) out = out.split(s).join('<secret>');
  return out
    .replace(/\b([a-z][a-z0-9+.-]*):\/\/[^\s"'<>]+/gi, (_, scheme: string) => `<${scheme.toLowerCase()} link>`)
    .replace(/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, '<id>')
    .replace(/[A-Za-z0-9_+/=-]{32,}/g, '<secret>');
}

export async function buildReport(checks?: Check[] | null): Promise<string> {
  const version = await getVersion().catch(() => '?');
  const s = app.status;
  const set = app.settings;
  const p = app.activeProfile;
  const lines = [
    `Kova ${version} · ${app.platform || 'unknown'}${app.tv ? ' (TV)' : ''}`,
    `Status: ${s.running ? 'connected' : 'not connected'}${s.core ? ` · ${s.core}${s.bridged.length ? ' + xray bridge' : ''}${set?.core.anti_dpi && s.sidecar ? ' · anti-DPI' : ''}` : ''}${s.tun_active ? ' · TUN' : ''}`,
  ];
  if (s.error) lines.push(`Error: ${s.error}`);
  if (s.warning) lines.push(`Warning: ${s.warning}`);
  if (set) {
    const c = set.core;
    lines.push(
      `Mode: ${c.mode} · system proxy ${set.system_proxy ? 'on' : 'off'} · TUN ${c.tun.enabled ? `on (${c.tun.stack})` : 'off'} · fake-ip ${c.dns.fake_ip ? 'on' : 'off'} · sniff ${c.sniff ? 'on' : 'off'} · ipv6 ${c.ipv6 ? 'on' : 'off'}`,
      `Routing data: ${c.geodata.kind} · presets: ${c.presets.join(', ') || 'none'} · own rules: ${c.custom_rules.length} · smart routing ${c.smart_routing ? 'on' : 'off'} · auto-switch ${set.auto_switch ? 'on' : 'off'}`,
    );
  }
  if (p) lines.push(`Profile: ${p.format}, ${p.node_count} nodes, core ${p.core}, ${p.source.kind}${p.warnings.length ? `, ${p.warnings.length} entries skipped` : ''}`);
  if (checks?.length) {
    lines.push('', 'Checks:');
    for (const c of checks) lines.push(`  ${c.state.padEnd(7)} ${c.id}${c.detail ? `: ${c.detail}` : ''}`);
  }
  const crash = await api.lastCrash().catch(() => null);
  if (crash) {
    lines.push('', `Last crash, ${new Date(crash.time * 1000).toISOString()}:`, ...crash.text.trim().split('\n').slice(0, 60));
  }
  const logs = app.logs.slice(-200);
  if (logs.length) {
    lines.push('', `Last ${logs.length} log lines:`);
    for (const l of logs) lines.push(`${l.time} ${l.level.slice(0, 4).toUpperCase()} ${l.source}: ${l.message}`);
  }
  const secrets = [set?.core.secret, set?.core.local_password, set?.device_id].filter((x): x is string => !!x);
  return redact(lines.join('\n'), secrets);
}

/** Copy the report and say what was left out. */
export async function copyReport(checks?: Check[] | null, done?: string) {
  const text = await buildReport(checks);
  await navigator.clipboard.writeText(text);
  if (done) app.notify(done);
}
