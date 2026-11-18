// Release Kova from main:
//   npm run release -- 0.3.0 [--changelog <file>] [--allow-jump] [--allow-no-tags] [--allow-unpushed] [--dry-run]
// Run it on main with everything committed and in step with origin/main, and with a
// signing key set up for git (user.signingkey). It checks the version against the
// versioning policy, sets it (scripts/bump-version.mjs), checks that the workspace still
// builds with the locked dependencies, commits "Set the version to <version>" and makes
// the signed annotated tag v<version>. It never pushes: it prints the push command, which
// starts the release workflow (a draft release to check first). When a step fails it
// prints how to undo what was done.
// --dry-run prints the commands without running them or changing any file; it reads the
// last fetched state of origin/main and the local tags (it fetches nothing), and exits 1
// when a real run would refuse.

import { execFileSync } from 'node:child_process';
import { UsageError, parseArgs } from './check-version.mjs';

const usage = 'usage: npm run release -- <major.minor.patch>[-beta.1] [--changelog <file>] [--allow-jump] [--allow-no-tags] [--allow-unpushed] [--dry-run]';
const fail = (why) => {
  console.error(`release: ${why}`);
  process.exit(1);
};
let opts;
try {
  opts = parseArgs(process.argv.slice(2), ['--dry-run', '--allow-unpushed']);
} catch (e) {
  if (!(e instanceof UsageError)) throw e;
  fail(`${e.message}\n${usage}`);
}
const { version, dryRun: dry } = opts;
if (!/^\d+\.\d+\.\d+(-beta\.\d+)?$/.test(version ?? '')) fail(usage);
const tag = `v${version}`;
// A dry run only reports what would stop the release.
const problems = [];
const problem = (why) => {
  if (!dry) fail(why);
  problems.push(why);
  console.warn(`release: would refuse: ${why}`);
};

// The release commit is dated in UTC, so it doesn't show the local time zone.
process.env.TZ = 'UTC0';
const node = process.execPath;
const read = (cmd, ...a) => execFileSync(cmd, a, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }).trim();
// Like read, but an unset config key gives '' instead of an error.
const config = (key) => {
  try {
    return execFileSync('git', ['config', '--get', key], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return '';
  }
};
const run = (cmd, ...a) => {
  console.log(`> ${cmd === node ? 'node' : cmd} ${a.join(' ')}`);
  if (!dry) execFileSync(cmd, a, { stdio: 'inherit' });
};
const flags = [
  ...(opts.changelog ? ['--changelog', opts.changelog] : []),
  ...(opts.allowJump ? ['--allow-jump'] : []),
  ...(opts.allowNoTags ? ['--allow-no-tags'] : []),
];

const branch = read('git', 'rev-parse', '--abbrev-ref', 'HEAD');
if (branch !== 'main') problem(`run it from main (this is ${branch})`);
if (read('git', 'status', '--porcelain')) problem('commit or stash the changes first');
if (!config('user.signingkey')) {
  problem('no signing key: set git config user.signingkey (and gpg.format for an SSH key); the tag must be signed');
} else {
  console.log(`signing with ${config('gpg.format') || 'openpgp'}`);
}

// Commits made on GitHub meanwhile (merged dependency updates) must be pulled first, and
// the tags must be up to date for the version check.
if (dry) console.log('(dry run: nothing is fetched, origin/main and the tags are as last fetched)');
else run('git', 'fetch', 'origin', 'main', '--tags');
try {
  const behind = Number(read('git', 'rev-list', '--count', 'HEAD..origin/main'));
  if (behind > 0) problem(`main is ${behind} commit(s) behind origin/main: pull first`);
  const ahead = read('git', 'log', '--oneline', 'origin/main..HEAD');
  if (ahead) {
    console.log(`Commits that are not pushed yet and will be pushed with the tag:\n${ahead}`);
    if (!opts.allowUnpushed) problem('main is ahead of origin/main: push or review those commits, or pass --allow-unpushed');
  }
} catch {
  problem('cannot compare with origin/main');
}

// check-version only reads, so a dry run runs it too.
console.log(`> node scripts/check-version.mjs ${[version, ...flags].join(' ')}`);
try {
  execFileSync(node, ['scripts/check-version.mjs', version, ...flags], { stdio: 'inherit' });
} catch {
  problem(`${version} does not pass the version check`);
}

// Where it stands, for the undo hint when a step fails.
let stage = 'start';
const undo = {
  start: 'Nothing was changed.',
  bumped: 'To undo: git checkout -- .',
  committed: 'To undo: git reset --hard HEAD~1',
  tagged: `To undo: git tag -d ${tag} && git reset --hard HEAD~1`,
};
try {
  // bump-version writes nothing unless every file is right, so a failure there changes nothing.
  run(node, 'scripts/bump-version.mjs', version, ...flags);
  stage = 'bumped';
  run(node, 'scripts/check-version.mjs', version, '--check-files', ...flags);
  run('cargo', 'check', '--workspace', '--locked');
  run('git', 'commit', '--all', '--message', `Set the version to ${version}`);
  stage = 'committed';
  run('git', 'tag', '-s', '-a', tag, '-m', tag);
  stage = 'tagged';
  run('git', 'tag', '-v', tag);
} catch (e) {
  fail(`a step failed (${String(e.message).split('\n')[0]}). ${undo[stage]}`);
}
if (dry && problems.length) {
  console.error(`\nrelease: dry run: a real run would refuse (${problems.length} problem(s) above).`);
  process.exit(1);
}
console.log(`\n${tag} ${dry ? 'would be' : 'is'} committed and tagged here, and not pushed. Check it, then push:`);
console.log(`  git push origin main ${tag}`);
console.log('The release workflow then builds a draft release: check it before you publish it.');
