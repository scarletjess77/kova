// Release Kova in one go: npm run release -- 0.2.0   (or 0.2.0-beta.1, or --dry-run)
// Run from the android branch with everything committed. It sets the version, commits
// "Release <version>", moves beta to it and, for a stable version, merges beta into
// main; then it tags v<version> and pushes. A pre-release (0.2.0-beta.1) is tagged on
// beta and doesn't reach main. The release workflow then builds a draft release.
// --dry-run prints the git commands without running them or changing the version.

import { execFileSync } from 'node:child_process';

const args = process.argv.slice(2);
const dry = args.includes('--dry-run');
const version = args.find((a) => !a.startsWith('--'));
const fail = (why) => {
  console.error(`release: ${why}`);
  process.exit(1);
};
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version ?? '')) fail('usage: npm run release -- <major.minor.patch>[-beta.1] [--dry-run]');
const pre = version.includes('-');
const tag = `v${version}`;

const here = process.cwd();
const read = (cwd, ...a) => execFileSync('git', a, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }).trim();
const run = (cwd, ...a) => {
  console.log(`> git ${a.join(' ')}    (${cwd})`);
  if (!dry) execFileSync('git', a, { cwd, stdio: 'inherit' });
};

// Each branch is checked out in a worktree of its own: android, beta, main.
const trees = {};
let tree;
for (const line of read(here, 'worktree', 'list', '--porcelain').split('\n')) {
  if (line.startsWith('worktree ')) tree = line.slice('worktree '.length);
  else if (line.startsWith('branch refs/heads/')) trees[line.slice('branch refs/heads/'.length)] = tree;
}
const branch = read(here, 'rev-parse', '--abbrev-ref', 'HEAD');
if (branch !== 'android') fail(`run it from the android branch (this is ${branch})`);
if (read(here, 'status', '--porcelain')) fail('commit or stash the changes first');
for (const b of pre ? ['beta'] : ['beta', 'main']) if (!trees[b]) fail(`no worktree has ${b} checked out`);
if (read(here, 'tag', '--list', tag)) fail(`${tag} exists already`);

if (dry) console.log(`> node scripts/bump-version.mjs ${version}`);
else execFileSync(process.execPath, ['scripts/bump-version.mjs', version], { cwd: here, stdio: 'inherit' });
run(here, 'commit', '--all', '--message', `Release ${version}`);
run(trees.beta, 'merge', '--ff-only', 'android');
const branches = ['beta'];
if (pre) {
  run(trees.beta, 'tag', tag);
} else {
  run(trees.main, 'merge', '--no-ff', 'beta', '--message', `Merge beta: ${version}`);
  run(trees.main, 'tag', tag);
  branches.push('main');
}
run(here, 'push', 'origin', ...branches, tag);
console.log(`\n${tag} ${dry ? 'would be' : 'is'} pushed. The release workflow builds it as a draft release.`);
