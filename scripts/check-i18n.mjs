// Checks the Russian dictionary against the texts in the UI: every t()/tn()/tx() text
// needs a translation, every translation must still be used, counted texts need their
// plural forms, and a text and its translation must have the same {placeholders}.
// Run by `npm run check`.

import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = join(import.meta.dirname, '..', 'ui', 'src');
const STR = String.raw`'((?:[^'\\]|\\.)*)'`;
const re = (s, flags = 'g') => new RegExp(s, flags);

function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? (e.name === 'lang' ? [] : files(join(dir, e.name))) : /\.(svelte|ts)$/.test(e.name) ? [join(dir, e.name)] : [],
  );
}

const used = new Map(); // text -> { file, counted }
for (const f of files(root)) {
  const name = relative(root, f).replaceAll('\\', '/');
  if (name === 'lib/i18n.svelte.ts') continue;
  const src = readFileSync(f, 'utf8');
  for (const m of src.matchAll(re(String.raw`(?<![\w.$])t\(\s*${STR}`))) used.set(m[1], { file: name, counted: false });
  for (const m of src.matchAll(re(String.raw`(?<![\w.$])tx\(\s*${STR}\s*,\s*${STR}`))) used.set(`${m[1]}|${m[2]}`, { file: name, counted: false });
  for (const m of src.matchAll(re(String.raw`(?<![\w.$])tn\([^']*?${STR}\s*,\s*${STR}`))) used.set(m[2], { file: name, counted: true });
  // A text built at run time can't be checked here.
  for (const m of src.matchAll(/(?<![\w.$])tx?\(\s*(?!')/g)) console.log(`note: ${name}:${src.slice(0, m.index).split('\n').length} translates a computed text`);
}

const dict = new Map(); // text -> { vals, counted }
const ruSrc = readFileSync(join(root, 'lib', 'lang', 'ru.ts'), 'utf8');
for (const m of ruSrc.matchAll(re(String.raw`^\s*${STR}:\s*(\[[^\]]*\]|${STR})`, 'gm'))) {
  dict.set(m[1], { vals: [...m[2].matchAll(re(STR))].map((v) => v[1]), counted: m[2].startsWith('[') });
}

const holes = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((x) => x[1]).sort().join(',');
const problems = [];
for (const [text, u] of used) {
  const d = dict.get(text);
  if (!d) { problems.push(`missing: '${text}' (${u.file})`); continue; }
  if (u.counted !== d.counted) problems.push(u.counted ? `needs plural forms: '${text}'` : `has plural forms but isn't counted: '${text}'`);
  for (const v of d.vals) if (holes(v) !== holes(text)) problems.push(`placeholders differ: '${text}' -> '${v}'`);
}
for (const text of dict.keys()) if (!used.has(text)) problems.push(`unused: '${text}'`);

if (problems.length) {
  console.error(`${problems.join('\n')}\n\n${problems.length} problem(s) in ui/src/lib/lang/ru.ts`);
  process.exit(1);
}
console.log(`ru: all ${used.size} texts translated`);
