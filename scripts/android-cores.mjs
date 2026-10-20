// Fetch Mihomo and Xray for Android (arm64) into the APK's native libraries, as
// libmihomo.so and libxray.so: Android only runs programs that ship inside the app.
// Each download is checked against the SHA-256 digest GitHub publishes for it.
// Usage: node scripts/android-cores.mjs
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { gunzipSync, inflateRawSync } from 'node:zlib';

const out = join(import.meta.dirname, '..', 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'jniLibs', 'arm64-v8a');
const CORES = [
  { repo: 'MetaCubeX/mihomo', asset: /^mihomo-android-arm64-v8-v[\d.]+\.gz$/, lib: 'libmihomo.so', unpack: (data) => gunzipSync(data) },
  { repo: 'XTLS/Xray-core', asset: /^Xray-android-arm64-v8a\.zip$/, lib: 'libxray.so', unpack: (data) => fromZip(data, 'xray') },
];

/** One file out of a zip archive, found through the archive's central directory. */
function fromZip(zip, name) {
  const end = zip.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error('not a zip archive');
  let at = zip.readUInt32LE(end + 16);
  for (let n = zip.readUInt16LE(end + 10); n > 0; n--) {
    const method = zip.readUInt16LE(at + 10);
    const size = zip.readUInt32LE(at + 20);
    const nameLen = zip.readUInt16LE(at + 28);
    const skip = nameLen + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    if (zip.toString('utf8', at + 46, at + 46 + nameLen) === name) {
      const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
      const body = zip.subarray(start, start + size);
      return method === 0 ? body : inflateRawSync(body);
    }
    at += 46 + skip;
  }
  throw new Error(`${name} is not in the archive`);
}

async function github(path) {
  const r = await fetch(`https://api.github.com/${path}`, { headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'kova-build' } });
  if (!r.ok) throw new Error(`GitHub answered ${r.status} for ${path}`);
  return r.json();
}

mkdirSync(out, { recursive: true });
for (const core of CORES) {
  const rel = await github(`repos/${core.repo}/releases/latest`);
  const asset = rel.assets.find((a) => core.asset.test(a.name));
  if (!asset) throw new Error(`${core.repo} ${rel.tag_name} has no Android arm64 build`);
  if (!asset.digest?.startsWith('sha256:')) throw new Error(`${asset.name} has no published checksum`);
  const r = await fetch(asset.browser_download_url);
  if (!r.ok) throw new Error(`download of ${asset.name} failed: ${r.status}`);
  const data = Buffer.from(await r.arrayBuffer());
  const actual = createHash('sha256').update(data).digest('hex');
  if (`sha256:${actual}` !== asset.digest) throw new Error(`checksum mismatch for ${asset.name}`);
  writeFileSync(join(out, core.lib), core.unpack(data));
  console.log(`${core.lib}: ${core.repo} ${rel.tag_name} (${asset.name}, checksum ok)`);
}
