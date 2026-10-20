// Set the app version everywhere it is recorded: node scripts/bump-version.mjs 0.2.0
import { readFileSync, writeFileSync } from 'node:fs';

const v = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(v ?? '')) {
  console.error('usage: node scripts/bump-version.mjs <major.minor.patch>');
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
console.log(`Version set to ${v}. Commit, then tag and push: git tag v${v} && git push origin v${v}`);
