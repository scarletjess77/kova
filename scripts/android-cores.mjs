// Put Mihomo and Xray for Android (arm64) into the APK's native libraries, as
// libmihomo.so and libxray.so: Android only runs programs that ship inside the app.
//
// Xray is the official release, checked against the SHA-256 digest GitHub publishes.
// Mihomo is built from the official source of its latest release, with the `cmfa` tag
// that Clash Meta for Android uses: the released Android build is made for rooted phones
// and reads the system's package list whenever TUN starts, which an app may not do.
// That needs Go; Go checks every library against Mihomo's go.sum.
//
// Usage: node scripts/android-cores.mjs
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { inflateRawSync } from 'node:zlib';

const root = join(import.meta.dirname, '..');
const out = join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'jniLibs', 'arm64-v8a');

async function github(path) {
  const r = await fetch(`https://api.github.com/${path}`, { headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'kova-build' } });
  if (!r.ok) throw new Error(`GitHub answered ${r.status} for ${path}`);
  return r.json();
}

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

async function mihomo() {
  const { tag_name: tag } = await github('repos/MetaCubeX/mihomo/releases/latest');
  const src = join(root, 'target', 'android-src', `mihomo-${tag}`);
  if (!existsSync(src)) {
    execFileSync('git', ['clone', '--quiet', '--depth', '1', '--branch', tag, 'https://github.com/MetaCubeX/mihomo.git', src], { stdio: 'inherit' });
  }
  const ldflags = `-X "github.com/metacubex/mihomo/constant.Version=${tag}" -w -s`;
  execFileSync('go', ['build', '-tags', 'with_gvisor,cmfa', '-trimpath', '-ldflags', ldflags, '-o', join(out, 'libmihomo.so'), '.'], {
    cwd: src,
    stdio: 'inherit',
    env: { ...process.env, GOOS: 'android', GOARCH: 'arm64', CGO_ENABLED: '0' },
  });
  console.log(`libmihomo.so: MetaCubeX/mihomo ${tag}, built from source (cmfa)`);
}

async function xray() {
  const rel = await github('repos/XTLS/Xray-core/releases/latest');
  const asset = rel.assets.find((a) => a.name === 'Xray-android-arm64-v8a.zip');
  if (!asset) throw new Error(`Xray ${rel.tag_name} has no Android arm64 build`);
  if (!asset.digest?.startsWith('sha256:')) throw new Error(`${asset.name} has no published checksum`);
  const r = await fetch(asset.browser_download_url);
  if (!r.ok) throw new Error(`download of ${asset.name} failed: ${r.status}`);
  const data = Buffer.from(await r.arrayBuffer());
  if (`sha256:${createHash('sha256').update(data).digest('hex')}` !== asset.digest) throw new Error(`checksum mismatch for ${asset.name}`);
  writeFileSync(join(out, 'libxray.so'), fromZip(data, 'xray'));
  console.log(`libxray.so: XTLS/Xray-core ${rel.tag_name} (${asset.name}, checksum ok)`);
}

mkdirSync(out, { recursive: true });
await mihomo();
await xray();
