// Put Mihomo and Xray for Android (arm64) into the APK's native libraries, as
// libmihomo.so and libxray.so: Android only runs programs that ship inside the app.
//
// Xray is the official release, checked against the SHA-256 digest GitHub publishes.
// Mihomo is built from the official source of its latest release, with the `cmfa` tag
// that Clash Meta for Android uses: the released Android build is made for rooted phones
// and reads the system's package list whenever TUN starts, which an app may not do.
// That needs Go; Go checks every library against Mihomo's go.sum.
//
// The versions are pinned in android-cores.json, so a new core reaches the APK only as a
// reviewed change to that file. Mihomo's tag must also point at the pinned commit.
//
// Usage: node scripts/android-cores.mjs
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { inflateRawSync } from 'node:zlib';

const root = join(import.meta.dirname, '..');
const PINNED = JSON.parse(readFileSync(join(import.meta.dirname, 'android-cores.json'), 'utf8'));
const out = join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'jniLibs', 'arm64-v8a');

/** fetch, tried again a few times: a connection dropped halfway shouldn't fail a release. */
async function fetchRetrying(url, init) {
  for (let attempt = 1; ; attempt++) {
    try {
      const r = await fetch(url, init);
      if (r.status < 500 || attempt === 4) return r;
    } catch (e) {
      if (attempt === 4) throw e;
      console.warn(`${url}: ${e.cause?.message ?? e.message}, trying again`);
    }
    await new Promise((done) => setTimeout(done, attempt * 3000));
  }
}

async function github(path) {
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'kova-build' };
  // CI runners share addresses, and with them GitHub's limit for anonymous requests.
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const r = await fetchRetrying(`https://api.github.com/${path}`, { headers });
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
  const { tag, commit } = PINNED.mihomo;
  const src = join(root, 'target', 'android-src', `mihomo-${tag}`);
  if (!existsSync(src)) {
    // Into a temporary folder first: an interrupted clone must not be mistaken for a
    // finished one next time.
    const partial = `${src}.partial`;
    rmSync(partial, { recursive: true, force: true });
    execFileSync('git', ['clone', '--quiet', '--depth', '1', '--branch', tag, 'https://github.com/MetaCubeX/mihomo.git', partial], { stdio: 'inherit' });
    const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: partial, encoding: 'utf8' }).trim();
    if (head !== commit) throw new Error(`Mihomo ${tag} is at ${head}, not the pinned ${commit}; the tag was moved`);
    renameSync(partial, src);
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
  const rel = await github(`repos/XTLS/Xray-core/releases/tags/${PINNED.xray.tag}`);
  const asset = rel.assets.find((a) => a.name === 'Xray-android-arm64-v8a.zip');
  if (!asset) throw new Error(`Xray ${rel.tag_name} has no Android arm64 build`);
  if (!asset.digest?.startsWith('sha256:')) throw new Error(`${asset.name} has no published checksum`);
  const r = await fetchRetrying(asset.browser_download_url);
  if (!r.ok) throw new Error(`download of ${asset.name} failed: ${r.status}`);
  const data = Buffer.from(await r.arrayBuffer());
  if (`sha256:${createHash('sha256').update(data).digest('hex')}` !== asset.digest) throw new Error(`checksum mismatch for ${asset.name}`);
  writeFileSync(join(out, 'libxray.so'), fromZip(data, 'xray'));
  console.log(`libxray.so: XTLS/Xray-core ${rel.tag_name} (${asset.name}, checksum ok)`);
}

mkdirSync(out, { recursive: true });
// Xray first: Mihomo's build takes a minute, and GitHub closes idle connections meanwhile.
await xray();
await mihomo();
