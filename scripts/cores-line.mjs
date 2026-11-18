// One line for the release notes: node scripts/cores-line.mjs
// Prints which core versions the build ships, from the pins in scripts/android-cores.json.
import { readFileSync, realpathSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pins = join(dirname(fileURLToPath(import.meta.url)), 'android-cores.json');
const LABELS = [['mihomo', 'Mihomo'], ['xray', 'Xray'], ['shoes', 'Shoes']];

/** `Ships Mihomo v1.19.34, Xray v26.11.9 and Shoes v0.3.7.` from the parsed pin file. */
export function coresLine(cores) {
  const parts = LABELS.filter(([key]) => cores?.[key]?.tag).map(([key, label]) => `${label} ${cores[key].tag}`);
  if (!parts.length) throw new Error('no core is pinned');
  const list = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}`;
  return `Ships ${list}.`;
}

const isMain = () => {
  try {
    return realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(process.argv[1]));
  } catch {
    return false;
  }
};

if (isMain()) console.log(coresLine(JSON.parse(readFileSync(pins, 'utf8'))));
