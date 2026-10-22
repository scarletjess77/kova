// Set the app version everywhere it is recorded: node scripts/bump-version.mjs 0.2.0
// A pre-release such as 0.2.0-alpha.1 is published as a GitHub pre-release, which
// installed copies don't update to.
import { readFileSync, writeFileSync } from 'node:fs';

const v = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(v ?? '')) {
  console.error('usage: node scripts/bump-version.mjs <major.minor.patch>[-alpha.1]');
  process.exit(1);
}
const edit = (file, from, to) => {
  const text = readFileSync(file, 'utf8');
  if (!from.test(text)) throw new Error(`no version found in ${file}`);
  writeFileSync(file, text.replace(from, to));
};
edit('package.json', /"version": "[^"]+"/, `"version": "${v}"`);
// The lockfile repeats the root version twice; `npm ci` wants them in step.
let lock = readFileSync('package-lock.json', 'utf8');
for (let i = 0; i < 2; i++) lock = lock.replace(/("name": "kova",\s*"version": )"(?!__V__)[^"]+"/, `$1"__V__"`);
writeFileSync('package-lock.json', lock.replaceAll('"__V__"', `"${v}"`));
edit('src-tauri/tauri.conf.json', /"version": "[^"]+"/, `"version": "${v}"`);
edit('Cargo.toml', /(\[workspace\.package\][^[]*?version = )"[^"]+"/, `$1"${v}"`);
// Cargo.lock records the version of each of our own crates too.
edit('Cargo.lock', /(name = "kova(?:-core|-engine|-service)?"\r?\nversion = )"[^"]+"/g, `$1"${v}"`);
console.log(`Version set to ${v}. Commit, then tag and push: git tag v${v} && git push origin v${v}`);
