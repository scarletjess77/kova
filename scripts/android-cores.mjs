// Put Mihomo and Xray for Android into the APK's native libraries, as libmihomo.so and
// libxray.so: Android only runs programs that ship inside the app.
//
// Xray for arm64 is the official release, checked against the SHA-256 digest GitHub
// publishes. Xray has no 32-bit Android release, so for armeabi-v7a (older phones and
// most TVs) it is built from its official source at the pinned commit, like Mihomo.
// Mihomo is built from the official source of its latest release, with the `cmfa` tag
// that Clash Meta for Android uses: the released Android build is made for rooted phones
// and reads the system's package list whenever TUN starts, which an app may not do.
// That needs Go; Go checks every library against Mihomo's go.sum.
//
// The versions are pinned in android-cores.json, so a new core reaches the APK only as a
// reviewed change to that file. Mihomo's tag must also point at the pinned commit.
//
// Usage: node scripts/android-cores.mjs [arm64-v8a] [armeabi-v7a]   (default: arm64-v8a)
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { inflateRawSync } from 'node:zlib';

const root = join(import.meta.dirname, '..');
const PINNED = JSON.parse(readFileSync(join(import.meta.dirname, 'android-cores.json'), 'utf8'));
const jniLibs = join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'jniLibs');
// Go's names for each Android ABI.
const GO = { 'arm64-v8a': { GOARCH: 'arm64' }, 'armeabi-v7a': { GOARCH: 'arm', GOARM: '7' } };

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

/** A pinned release's source, cloned once and checked against the pinned commit. */
function source(name, repo, tag, commit) {
  const src = join(root, 'target', 'android-src', `${name}-${tag}`);
  if (!existsSync(src)) {
    // Into a temporary folder first: an interrupted clone must not be mistaken for a
    // finished one next time.
    const partial = `${src}.partial`;
    rmSync(partial, { recursive: true, force: true });
    execFileSync('git', ['clone', '--quiet', '--depth', '1', '--branch', tag, `https://github.com/${repo}.git`, partial], { stdio: 'inherit' });
    const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: partial, encoding: 'utf8' }).trim();
    if (head !== commit) throw new Error(`${repo} ${tag} is at ${head}, not the pinned ${commit}; the tag was moved`);
    renameSync(partial, src);
  }
  return src;
}

/**
 * Go links 32-bit Android programs only through the NDK's C compiler (`NDK_HOME`);
 * 64-bit ones need no C at all.
 */
function cgoEnv(abi) {
  if (abi === 'arm64-v8a') return { CGO_ENABLED: '0' };
  const ndk = process.env.NDK_HOME;
  if (!ndk) throw new Error('Set NDK_HOME to build for 32-bit ARM');
  const host = process.platform === 'win32' ? 'windows-x86_64' : process.platform === 'darwin' ? 'darwin-x86_64' : 'linux-x86_64';
  const cc = join(ndk, 'toolchains', 'llvm', 'prebuilt', host, 'bin', `armv7a-linux-androideabi24-clang${process.platform === 'win32' ? '.cmd' : ''}`);
  return { CGO_ENABLED: '1', CC: cc };
}

function goBuild(src, abi, args, output) {
  execFileSync('go', ['build', '-trimpath', ...args, '-o', output, '.'], {
    cwd: src,
    stdio: 'inherit',
    env: { ...process.env, GOOS: 'android', ...GO[abi], ...cgoEnv(abi) },
  });
}

async function mihomo(abi) {
  const { tag, commit } = PINNED.mihomo;
  const src = source('mihomo', 'MetaCubeX/mihomo', tag, commit);
  const ldflags = `-X "github.com/metacubex/mihomo/constant.Version=${tag}" -w -s`;
  goBuild(src, abi, ['-tags', 'with_gvisor,cmfa', '-ldflags', ldflags], join(jniLibs, abi, 'libmihomo.so'));
  console.log(`${abi}/libmihomo.so: MetaCubeX/mihomo ${tag}, built from source (cmfa)`);
}

async function xray(abi) {
  if (abi !== 'arm64-v8a') {
    const { tag, commit } = PINNED.xray;
    const src = source('xray', 'XTLS/Xray-core', tag, commit);
    goBuild(join(src, 'main'), abi, ['-ldflags', '-s -w -buildid='], join(jniLibs, abi, 'libxray.so'));
    console.log(`${abi}/libxray.so: XTLS/Xray-core ${tag}, built from source`);
    return;
  }
  const rel = await github(`repos/XTLS/Xray-core/releases/tags/${PINNED.xray.tag}`);
  const asset = rel.assets.find((a) => a.name === 'Xray-android-arm64-v8a.zip');
  if (!asset) throw new Error(`Xray ${rel.tag_name} has no Android arm64 build`);
  if (!asset.digest?.startsWith('sha256:')) throw new Error(`${asset.name} has no published checksum`);
  const r = await fetchRetrying(asset.browser_download_url);
  if (!r.ok) throw new Error(`download of ${asset.name} failed: ${r.status}`);
  const data = Buffer.from(await r.arrayBuffer());
  if (`sha256:${createHash('sha256').update(data).digest('hex')}` !== asset.digest) throw new Error(`checksum mismatch for ${asset.name}`);
  writeFileSync(join(jniLibs, abi, 'libxray.so'), fromZip(data, 'xray'));
  console.log(`${abi}/libxray.so: XTLS/Xray-core ${rel.tag_name} (${asset.name}, checksum ok)`);
}

const abis = process.argv.slice(2).length ? process.argv.slice(2) : ['arm64-v8a'];
for (const abi of abis) {
  if (!GO[abi]) throw new Error(`unknown ABI ${abi}; use ${Object.keys(GO).join(' or ')}`);
  mkdirSync(join(jniLibs, abi), { recursive: true });
  // Xray first: Mihomo's build takes a minute, and GitHub closes idle connections meanwhile.
  await xray(abi);
  await mihomo(abi);
}
