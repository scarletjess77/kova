// Writes latest.json, the desktop updater's manifest, for a release from its signed
// packages, uploads it, and removes the signature files: they are inside latest.json then,
// and on the release page they only confuse. One job does this after every desktop build
// is done: when each build added its own part, two finishing together overwrote each other's.
// Usage: node scripts/updater-json.mjs v1.2.3   (needs gh, GH_TOKEN, GITHUB_REPOSITORY)

import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const tag = process.argv[2];
const repo = process.env.GITHUB_REPOSITORY;
if (!tag || !repo) throw new Error('usage: GITHUB_REPOSITORY=owner/name node scripts/updater-json.mjs <tag>');
const gh = (...args) => execFileSync('gh', [...args, '-R', repo], { encoding: 'utf8' });
const download = (name) => gh('release', 'download', tag, '-p', name, '-O', '-');

const assets = JSON.parse(gh('release', 'view', tag, '--json', 'assets')).assets.map((a) => a.name);

// The updater looks up "<os>-<arch>-<installer>" first, then "<os>-<arch>"; one Mac app
// serves both processors. The second names are the ones releases had before 0.1.18.
const PACKAGES = {
  'windows-x86_64': /_(Windows|x64-setup)\.exe$/,
  'windows-x86_64-nsis': /_(Windows|x64-setup)\.exe$/,
  'darwin-aarch64': /_(macOS|aarch64)\.app\.tar\.gz$/,
  'darwin-aarch64-app': /_(macOS|aarch64)\.app\.tar\.gz$/,
  'darwin-x86_64': /_(macOS|x64)\.app\.tar\.gz$/,
  'darwin-x86_64-app': /_(macOS|x64)\.app\.tar\.gz$/,
  'linux-x86_64': /_(Linux|amd64)\.AppImage$/,
  'linux-x86_64-appimage': /_(Linux|amd64)\.AppImage$/,
  'linux-x86_64-deb': /_(Linux|amd64)\.deb$/,
};

// Re-running one build (say, a Mac one that found no machine) finds the other systems'
// signatures already folded into latest.json and their files gone: those entries carry over.
const previous = (assets.includes('latest.json') && JSON.parse(download('latest.json')).platforms) || {};

const signatures = {};
const platforms = {};
const missing = [];
for (const [key, pattern] of Object.entries(PACKAGES)) {
  const name = assets.find((n) => pattern.test(n));
  const url = name && `https://github.com/${repo}/releases/download/${tag}/${name}`;
  if (name && assets.includes(`${name}.sig`)) {
    signatures[name] ??= download(`${name}.sig`).trim();
    platforms[key] = { signature: signatures[name], url };
  } else if (url && previous[key]?.url === url) {
    platforms[key] = previous[key];
  } else {
    missing.push(key);
  }
}
if (!Object.keys(platforms).length) throw new Error('no signed packages in this release');
// A system whose build is missing (say, no Mac machine was free) goes without the update
// until its build is re-run, which writes this file again.
if (missing.length) console.warn(`no signed package yet for ${missing.join(', ')}`);

// Kova shows the release's own notes; they are written after the build.
const manifest = { version: tag.replace(/^v/, ''), notes: '', pub_date: new Date().toISOString(), platforms };
writeFileSync('latest.json', `${JSON.stringify(manifest, null, 2)}\n`);
gh('release', 'upload', tag, 'latest.json', '--clobber');
for (const sig of assets.filter((n) => n.endsWith('.sig'))) gh('release', 'delete-asset', tag, sig, '--yes');
console.log(`latest.json for ${tag}: ${Object.keys(platforms).join(', ')}`);
