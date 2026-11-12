// Writes into android-cores.json the SHA-256 of every file Kova may download for the
// pinned core versions. Kova's builds and the app then install a pinned core only when
// the download has exactly that checksum: a release whose files were replaced after the
// pin was reviewed is refused, even though GitHub would publish a matching digest for
// the new files.
//
// Run it after changing a tag in android-cores.json, and review the checksums it writes
// like the tag itself.
// Usage: node scripts/pin-cores.mjs   (GITHUB_TOKEN is used when set)

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const file = join(import.meta.dirname, 'android-cores.json');
const pins = JSON.parse(readFileSync(file, 'utf8'));

// The files Kova picks from (see `pick_asset` in crates/kova-engine/src/cores.rs, and
// android-cores.mjs for Xray on Android).
const CORES = {
  mihomo: { repo: 'MetaCubeX/mihomo', files: /^mihomo-(windows|linux|darwin)-(amd64(-compatible|-v1)?|arm64)-v[\d.]+\.(zip|gz)$/ },
  xray: { repo: 'XTLS/Xray-core', files: /^Xray-(windows|linux|macos|android)-(64|arm64-v8a)\.zip$/ },
  shoes: { repo: 'cfal/shoes', files: /^shoes-(aarch64|x86_64)-(unknown-linux-musl|apple-darwin)\.tar\.gz$/ },
};

for (const [name, { repo, files }] of Object.entries(CORES)) {
  const { tag } = pins[name];
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'kova-build' };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const r = await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers });
  if (!r.ok) throw new Error(`GitHub answered ${r.status} for ${repo} ${tag}`);
  const sha256 = {};
  for (const asset of (await r.json()).assets.filter((a) => files.test(a.name)).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!/^sha256:[0-9a-f]{64}$/.test(asset.digest ?? '')) throw new Error(`${repo} ${tag}: ${asset.name} has no published SHA-256`);
    sha256[asset.name] = asset.digest.slice('sha256:'.length);
  }
  if (!Object.keys(sha256).length) throw new Error(`${repo} ${tag} has none of the files Kova uses`);
  pins[name].sha256 = sha256;
  console.log(`${name} ${tag}: ${Object.keys(sha256).length} files`);
}

writeFileSync(file, `${JSON.stringify(pins, null, 2)}\n`);
