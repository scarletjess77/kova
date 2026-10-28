// Writes latest.json, the desktop updater's manifest, for a release from its signed
// packages, and uploads it. One job does this after every desktop build is done: when
// each build added its own part, two finishing together overwrote each other's.
// Usage: node scripts/updater-json.mjs v1.2.3   (needs gh, GH_TOKEN, GITHUB_REPOSITORY)

import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const tag = process.argv[2];
const repo = process.env.GITHUB_REPOSITORY;
if (!tag || !repo) throw new Error('usage: GITHUB_REPOSITORY=owner/name node scripts/updater-json.mjs <tag>');
const gh = (...args) => execFileSync('gh', [...args, '-R', repo], { encoding: 'utf8' });

const assets = JSON.parse(gh('release', 'view', tag, '--json', 'assets')).assets.map((a) => a.name);

// The updater looks up "<os>-<arch>-<installer>" first, then "<os>-<arch>".
const PACKAGES = {
  'windows-x86_64': /_x64-setup\.exe$/,
  'windows-x86_64-nsis': /_x64-setup\.exe$/,
  'darwin-aarch64': /_aarch64\.app\.tar\.gz$/,
  'darwin-aarch64-app': /_aarch64\.app\.tar\.gz$/,
  'darwin-x86_64': /_x64\.app\.tar\.gz$/,
  'darwin-x86_64-app': /_x64\.app\.tar\.gz$/,
  'linux-x86_64': /_amd64\.AppImage$/,
  'linux-x86_64-appimage': /_amd64\.AppImage$/,
  'linux-x86_64-deb': /_amd64\.deb$/,
};

const platforms = {};
const missing = [];
for (const [key, pattern] of Object.entries(PACKAGES)) {
  const name = assets.find((n) => pattern.test(n));
  if (!name || !assets.includes(`${name}.sig`)) {
    missing.push(key);
    continue;
  }
  platforms[key] = {
    signature: gh('release', 'download', tag, '-p', `${name}.sig`, '-O', '-').trim(),
    url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
  };
}
if (missing.length) throw new Error(`no signed package for ${missing.join(', ')}`);

// Kova shows the release's own notes; they are written after the build.
const manifest = { version: tag.replace(/^v/, ''), notes: '', pub_date: new Date().toISOString(), platforms };
writeFileSync('latest.json', `${JSON.stringify(manifest, null, 2)}\n`);
gh('release', 'upload', tag, 'latest.json', '--clobber');
console.log(`latest.json for ${tag}: ${Object.keys(platforms).join(', ')}`);
