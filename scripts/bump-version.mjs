// Set the app version everywhere it is recorded:
//   node scripts/bump-version.mjs 0.3.0 [--changelog <file>] [--allow-jump] [--allow-no-tags] [--force]
// The version is checked against the versioning policy first (scripts/check-version.mjs):
// X.Y.Z or X.Y.Z-beta.N, one step after the latest stable tag, and so on. Nothing is
// written when the check fails; --force skips it. All five files are worked out in
// memory first and written only when every one has what it should, so a failure leaves
// them all as they were. A beta is published as a GitHub pre-release, which installed
// copies see only with the beta channel setting on.
import { readFileSync, writeFileSync } from 'node:fs';
import { VERSION_FILES, UsageError, applyVersion, parseArgs, printResult, runCheck } from './check-version.mjs';

const usage = 'usage: node scripts/bump-version.mjs <major.minor.patch>[-beta.1] [--changelog <file>] [--allow-jump] [--allow-no-tags] [--force]';
let opts;
try {
  opts = parseArgs(process.argv.slice(2), ['--force']);
} catch (e) {
  if (!(e instanceof UsageError)) throw e;
  console.error(`bump-version: ${e.message}\n${usage}`);
  process.exit(1);
}
const v = opts.version;
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(v ?? '')) {
  console.error(usage);
  process.exit(1);
}
if (opts.force) {
  console.warn(`WARNING: --force skips the version check: ${v} is not checked against the versioning policy.`);
} else {
  const result = runCheck({ ...opts, checkFiles: false, ci: false });
  printResult(v, result);
  if (!result.ok) {
    console.error(`\nbump-version: ${v} refused, nothing was changed.`);
    process.exit(1);
  }
}
let next;
try {
  const files = Object.fromEntries(VERSION_FILES.map((f) => [f, readFileSync(f, 'utf8')]));
  next = applyVersion(files, v);
} catch (e) {
  console.error(`bump-version: ${e.message}\nNothing was changed.`);
  process.exit(1);
}
for (const file of VERSION_FILES) writeFileSync(file, next[file]);
console.log(`Version set to ${v}. Commit it, then tag it with a signed annotated tag: git tag -s -a v${v} -m "v${v}"`);
console.log(`Verify the signature with git tag -v v${v}, then push: git push origin main v${v}`);
