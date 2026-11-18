// Tests of the version policy: node --test scripts/check-version.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  UsageError, applyVersion, check, checkFiles, highest, nextSteps, parse, parseArgs, requiredBump, stableTags,
  verifyFiles, versionCode,
} from './check-version.mjs';
import { coresLine } from './cores-line.mjs';
import { readFileSync } from 'node:fs';

const TAGS = ['v0.1.9', 'v0.1.10', 'v0.1.36', 'v0.2.0', 'v0.2.9', 'v0.2.10'];
const run = (version, extra = {}) => check({ version, tags: TAGS, ...extra });
const has = (r, text) => r.errors.some((e) => e.includes(text));
const crlf = (s) => s.replace(/\n/g, '\r\n');

test('format: accepted and rejected versions', () => {
  for (const ok of ['0.2.10', '0.2.10-beta.1', '1.0.0', '0.3.0-beta.12']) assert.ok(parse(ok), ok);
  for (const bad of ['0.2.10-alpha.1', '1.0', 'v0.2.10', '0.2.10-beta', '0.2.10-beta.0', '0.2.10-beta.1.2', '01.2.3', '', undefined]) {
    assert.equal(parse(bad), null, String(bad));
  }
  const r = run('0.2.10-alpha.1');
  assert.equal(r.ok, false);
  assert.match(r.errors[0], /not a valid version/);
  assert.match(run('v0.2.11').errors[0], /not a valid version/);
  assert.match(run('1.0').errors[0], /not a valid version/);
  assert.match(run('0.2.10-beta').errors[0], /not a valid version/);
});

test('tags: numeric order, betas and other tags ignored', () => {
  const list = ['v0.1.9', 'v0.1.10', 'v0.1.36', 'v0.2.0', 'v0.2.10', 'v0.2.9', 'v0.3.0-beta.1', 'nightly', 'v1.0', 'vx.y.z', 'latest'];
  assert.deepEqual(stableTags(list).map((v) => v.text), ['0.1.9', '0.1.10', '0.1.36', '0.2.0', '0.2.10', '0.2.9']);
  assert.equal(highest(stableTags(list)).text, '0.2.10');
  assert.equal(highest(stableTags(['v0.1.9', 'v0.1.10', 'v0.1.36'])).text, '0.1.36');
  assert.equal(highest(stableTags(['v0.9.0', 'v0.10.0', 'v0.2.0'])).text, '0.10.0');
  assert.equal(highest([]), null);
  assert.deepEqual(nextSteps(parse('0.2.10')), ['0.2.11', '0.3.0', '1.0.0']);
});

test('the repository tag order is numeric, not alphabetical', () => {
  // Alphabetically v0.2.9 comes after v0.2.10, so 0.2.11 would follow 0.2.9 and fail.
  for (const tags of [['v0.2.10', 'v0.2.9'], ['v0.2.9', 'v0.2.10'], [...TAGS].reverse(), TAGS]) {
    const r = check({ version: '0.2.11', tags });
    assert.equal(r.info.latest, '0.2.10');
    assert.equal(r.ok, true, r.errors.join('; '));
  }
  assert.ok(has(check({ version: '0.2.10', tags: ['v0.2.9', 'v0.2.10'] }), 'exists already'));
  assert.ok(has(check({ version: '0.1.37', tags: TAGS }), 'not above the latest stable 0.2.10'));
});

test('one step after the latest stable version', () => {
  assert.equal(run('0.2.11').ok, true);
  assert.equal(run('0.3.0').ok, true);
  assert.equal(run('0.3.0').info.bump, 'minor');
  assert.equal(run('0.2.11').info.bump, 'patch');
  for (const jump of ['1.0.0', '0.3.1', '0.2.12', '0.30.0', '0.2.100']) {
    const r = run(jump);
    assert.equal(r.ok, false, jump);
    assert.ok(has(r, 'is not one step after the latest stable 0.2.10'), jump);
  }
  assert.equal(run('1.0.0', { allowJump: true }).ok, true);
  assert.equal(run('0.3.1', { allowJump: true }).ok, true);
  assert.ok(run('1.0.0', { allowJump: true }).warnings.length > 0);
  assert.equal(run('1.0.0', { allowJump: true }).info.bump, 'major');
  // From 1.x the next major is an ordinary step.
  assert.equal(check({ version: '2.0.0', tags: ['v1.4.2'] }).ok, true);
});

test('a number not above the latest stable version always fails', () => {
  for (const low of ['0.2.10', '0.2.9', '0.1.99']) {
    for (const allowJump of [false, true]) {
      const r = run(low, { allowJump });
      assert.equal(r.ok, false, `${low} ${allowJump}`);
      assert.ok(has(r, 'not above the latest stable 0.2.10'), `${low} ${allowJump}`);
    }
  }
  assert.ok(has(run('0.2.10'), 'exists already'));
});

test('betas: only for X.Y.0, numbered from 1 without gaps or repeats', () => {
  assert.equal(run('0.3.0-beta.1').ok, true);
  assert.equal(run('0.3.0-beta.1').info.betaBase, '0.3.0');
  assert.ok(has(run('0.3.0-beta.2'), 'skips a number'));
  assert.ok(has(run('0.2.11-beta.1'), 'only for an X.Y.0 release'));
  assert.ok(has(run('0.2.10-beta.1'), 'not above the latest stable'));
  assert.ok(has(run('0.3.1-beta.1'), 'only for an X.Y.0 release'));
  assert.ok(has(run('1.0.0-beta.1'), 'one step'));
  assert.equal(run('1.0.0-beta.1', { allowJump: true }).ok, true);

  const tags = [...TAGS, 'v0.3.0-beta.1', 'v0.3.0-beta.2'];
  assert.equal(check({ version: '0.3.0-beta.3', tags }).ok, true);
  assert.ok(has(check({ version: '0.3.0-beta.2', tags }), 'beta.2 of 0.3.0 exists already'));
  assert.ok(has(check({ version: '0.3.0-beta.1', tags }), 'exists already'));
  assert.ok(has(check({ version: '0.3.0-beta.4', tags }), 'skips a number'));
  assert.ok(has(check({ version: '0.3.0-beta.3', tags: [...TAGS, 'v0.3.0-beta.1'] }), 'skips a number'));
});

test('betas of one base do not count for another base', () => {
  const tags = [...TAGS, 'v0.3.0-beta.1'];
  // Their own first beta, whatever exists for 0.3.0.
  assert.equal(check({ version: '1.0.0-beta.1', tags, allowJump: true }).ok, true);
  assert.equal(check({ version: '0.3.0-beta.2', tags }).ok, true);
  assert.ok(has(check({ version: '0.4.0-beta.1', tags }), 'is not one step after'));
  // 1.0.0-beta.2 is a gap because 1.0.0 has no beta.1, whatever 0.3.0 has.
  assert.ok(has(check({ version: '1.0.0-beta.2', tags, allowJump: true }), 'skips a number'));
});

test('the final release after betas names the last beta', () => {
  const tags = [...TAGS, 'v0.3.0-beta.1', 'v0.3.0-beta.2'];
  const r = check({ version: '0.3.0', tags });
  assert.equal(r.ok, true, r.errors.join('; '));
  assert.equal(r.info.lastBeta, '0.3.0-beta.2');
  assert.equal(run('0.3.0').info.lastBeta, null);
  assert.equal(check({ version: '0.2.11', tags }).info.lastBeta, null);
});

test('versionCode: formula, increase and limits', () => {
  assert.equal(versionCode('0.2.5'), 2005);
  assert.equal(versionCode('0.2.10'), 2010);
  assert.equal(versionCode('0.3.0'), 3000);
  assert.equal(versionCode('1.0.0'), 1_000_000);
  assert.equal(versionCode('0.3.0-beta.2'), 3000);
  const r = run('0.3.0');
  assert.equal(r.info.versionCode, 3000);
  assert.equal(r.info.previousCode, 2010);
  assert.ok(has(run('0.2.1000'), 'patch version 1000 is above 999'));
  assert.ok(has(run('0.2.1000', { allowJump: true }), 'above 999'));
  assert.ok(has(run('0.1000.0', { allowJump: true }), 'minor version 1000 is above 999'));
  assert.ok(has(run('2001.0.0', { allowJump: true }), 'above 2000'));
  assert.equal(run('2000.0.0', { allowJump: true }).ok, true);
  assert.equal(check({ version: '0.2.999', tags: ['v0.2.998'] }).ok, true);
  // A tag with a huge patch number makes a lower minor look higher in versionCode.
  const odd = check({ version: '0.2.0', tags: ['v0.1.2000'] });
  assert.ok(has(odd, 'versionCode 2000 is not above 3000'));
});

test('stable tags only: betas do not set the latest version', () => {
  const r = check({ version: '0.2.11', tags: [...TAGS, 'v0.9.0-beta.4', 'v0.2.11-beta.1'] });
  assert.equal(r.info.latest, '0.2.10');
  assert.equal(r.ok, true);
});

test('no stable tag is an error unless allowed', () => {
  const r = check({ version: '0.1.0', tags: [] });
  assert.equal(r.ok, false);
  assert.ok(has(r, 'no stable tag found'));
  assert.ok(has(check({ version: '0.1.0', tags: ['v0.1.0-beta.1', 'nightly'] }), 'no stable tag found'));
  const allowed = check({ version: '0.1.0', tags: [], allowNoTags: true });
  assert.equal(allowed.ok, true);
  assert.equal(allowed.warnings.length, 1);
  // CI keeps its old behaviour: a warning.
  assert.equal(check({ version: '0.1.0', tags: ['v0.1.0'], ci: true }).ok, true);
});

const notes = (version, body) => `## Kova ${version}\n\n${body}`;
const ADDED = notes('0.3.0', '### Added\n\n  * A new thing.\n\n### Fixed\n\n  * A fix.\n');
const FIXED = notes('0.2.11', '### Fixed\n\n- A fix.\n\n### Security\n\n* A hole.\n');
const EMPTY = notes('0.2.11', '### Fixed\n\n### Added\n');
const BREAKING = notes('1.0.1', '### Fixed\n\n  * A fix.\n\n### Breaking\n\n  * The backup format changed.\n');

test('changelog: the least bump the notes call for', () => {
  assert.equal(requiredBump(ADDED).bump, 'minor');
  assert.equal(requiredBump(FIXED).bump, 'patch');
  assert.equal(requiredBump(EMPTY).bump, null);
  assert.equal(requiredBump('### Changed\r\n  * x\r\n').bump, 'minor');
  assert.equal(requiredBump(BREAKING, { major: 0 }).bump, 'minor');
  assert.equal(requiredBump(BREAKING, { major: 1 }).bump, 'major');
  assert.equal(requiredBump('### Security\n  * x\n').bump, 'patch');

  const patchWithAdded = run('0.2.11', { changelog: notes('0.2.11', '### Added\n  * x\n') });
  assert.equal(patchWithAdded.ok, false);
  assert.ok(has(patchWithAdded, 'has Added/Changed entries, so this release must be a minor (it is a patch)'));
  assert.equal(run('0.3.0', { changelog: ADDED }).ok, true);
  assert.equal(run('0.2.11', { changelog: FIXED }).ok, true);
  const minorWithFixed = run('0.3.0', { changelog: notes('0.3.0', '### Fixed\n  * a\n') });
  assert.equal(minorWithFixed.ok, true);
  assert.equal(minorWithFixed.warnings.length, 1);
  const empty = run('0.2.11', { changelog: EMPTY });
  assert.equal(empty.ok, false);
  assert.ok(has(empty, 'the changelog has no entries'));
  // Betas are checked against the bump of their base.
  assert.equal(run('0.3.0-beta.1', { changelog: ADDED }).ok, true);
  assert.equal(run('0.3.0-beta.1', { changelog: notes('0.3.0', '### Fixed\n  * a\n') }).warnings.length, 1);
});

test('changelog: breaking changes from 1.0 on need a major', () => {
  const tags = ['v1.0.0'];
  const minor = check({ version: '1.1.0', tags, changelog: notes('1.1.0', '### Breaking\n  * x\n') });
  assert.equal(minor.ok, false);
  assert.ok(has(minor, 'must be a major (it is a minor)'));
  assert.equal(check({ version: '2.0.0', tags, changelog: notes('2.0.0', '### Breaking\n  * x\n') }).ok, true);
  // Before 1.0 it is only a minor.
  assert.equal(run('0.3.0', { changelog: notes('0.3.0', '### Breaking\n  * x\n') }).ok, true);
  assert.equal(requiredBump('### Breaking changes\n  * x\n', { major: 1 }).bump, 'major');
});

test('changelog: CRLF, tabs, comments and rules', () => {
  const text = crlf(notes('0.2.11', '### Fixed\n\n\t* tabbed fix\n'));
  assert.equal(requiredBump(text).bump, 'patch');
  assert.equal(run('0.2.11', { changelog: text }).ok, true);
  // A bullet in a comment (also over several lines) is no entry.
  const commented = notes('0.3.0', '### Added\n<!--\n- placeholder\n-->\n### Fixed\n  * x <!-- - y -->\n');
  assert.equal(requiredBump(commented).counts.minor, 0);
  assert.equal(requiredBump(commented).bump, 'patch');
  assert.ok(has(run('0.3.0', { changelog: notes('0.3.0', '### Added\n<!--\n- placeholder\n-->\n') }), 'no entries'));
  // Horizontal rules are no bullets.
  const rules = notes('0.3.0', '### Added\n* * *\n---\n***\n- - -\n');
  assert.equal(requiredBump(rules).bump, null);
});

test('changelog: known and unknown sections', () => {
  assert.equal(requiredBump('### Removed\n  * x\n').bump, 'minor');
  assert.equal(requiredBump('### Deprecated\n  * x\n').bump, 'minor');
  assert.equal(requiredBump('###   FIXED  \n  * x\n').bump, 'patch');
  const odd = requiredBump('### Misc\n  * x\n### Fixed\n  * y\n');
  assert.deepEqual(odd.errors, ['unknown section Misc: use Added, Changed, Removed, Deprecated, Breaking, Fixed or Security']);
  const r = run('0.2.11', { changelog: notes('0.2.11', '### Misc\n  * x\n### Fixed\n  * y\n') });
  assert.equal(r.ok, false);
  assert.ok(has(r, 'unknown section Misc'));
  // An unknown heading without bullets is harmless.
  assert.deepEqual(requiredBump('### Misc\n\n### Fixed\n  * y\n').errors, []);
  // Lines under no section are ignored.
  assert.equal(requiredBump('## Kova 0.2.11\n  * stray\n').bump, null);
});

test('changelog: the heading names the version, and the first release only counts', () => {
  const wrong = run('0.3.0', { changelog: notes('0.3.1', '### Added\n  * x\n') });
  assert.equal(wrong.ok, false);
  assert.ok(has(wrong, 'does not name 0.3.0'));
  assert.ok(has(run('0.3.1', { changelog: notes('0.3.10', '### Fixed\n  * x\n'), allowJump: true }), 'does not name 0.3.1'));
  // A beta is compared with its base.
  assert.equal(run('0.3.0-beta.1', { changelog: notes('0.3.0', '### Added\n  * x\n') }).ok, true);
  assert.equal(run('0.2.11', { changelog: notes('v0.2.11 (2026-05-01)', '### Fixed\n  * x\n') }).ok, true);
  // No heading: a warning, and the version is not checked.
  const bare = run('0.2.11', { changelog: '### Fixed\n  * x\n' });
  assert.equal(bare.ok, true);
  assert.equal(bare.warnings.length, 1);
  // The notes of older releases below the first heading do not count.
  const two = notes('0.2.11', '### Fixed\n  * a\n\n## Kova 0.2.10\n\n### Added\n  * old\n  * older\n');
  assert.equal(requiredBump(two).bump, 'patch');
  assert.equal(run('0.2.11', { changelog: two }).ok, true);
});

test('--ci: the tag being built does not count against itself', () => {
  const tags = [...TAGS, 'v0.2.11'];
  assert.ok(has(check({ version: '0.2.11', tags }), 'exists already'));
  const r = check({ version: '0.2.11', tags, ci: true });
  assert.equal(r.ok, true, r.errors.join('; '));
  assert.equal(r.info.latest, '0.2.10');
  // The one-step rule is skipped, the order is not.
  assert.equal(check({ version: '0.5.0', tags: [...TAGS, 'v0.5.0'], ci: true }).ok, true);
  assert.ok(has(check({ version: '0.2.9', tags: [...TAGS], ci: true }), 'not above the latest stable 0.2.10'));
  // The build of the latest tag itself passes: it is compared with the ones before it.
  assert.equal(check({ version: '0.2.10', tags: [...TAGS], ci: true }).ok, true);
  assert.ok(has(check({ version: '0.2.1000', tags: [...TAGS, 'v0.2.1000'], ci: true }), 'above 999'));
  assert.ok(has(check({ version: '0.2.11-beta.1', tags: [...TAGS, 'v0.2.11-beta.1'], ci: true }), 'only for an X.Y.0 release'));
  // Beta numbers still count in CI.
  const betas = [...TAGS, 'v0.3.0-beta.1', 'v0.3.0-beta.2'];
  assert.equal(check({ version: '0.3.0-beta.2', tags: betas, ci: true }).ok, true);
  assert.equal(check({ version: '0.3.0-beta.1', tags: betas, ci: true }).ok, true);
  assert.ok(has(check({ version: '0.3.0-beta.3', tags: [...TAGS, 'v0.3.0-beta.1', 'v0.3.0-beta.3'], ci: true }), 'skips a number'));
  assert.equal(check({ version: '0.3.0', tags: [...betas, 'v0.3.0'], ci: true }).ok, true);
});

test('the command line options', () => {
  const o = parseArgs(['0.3.0', '--changelog=notes.md', '--allow-jump', '--allow-no-tags']);
  assert.equal(o.version, '0.3.0');
  assert.equal(o.changelog, 'notes.md');
  assert.equal(o.allowJump, true);
  assert.equal(o.allowNoTags, true);
  assert.equal(parseArgs(['--changelog', 'n.md', '0.3.0']).changelog, 'n.md');
  assert.equal(parseArgs(['0.3.0']).allowJump, false);
  assert.equal(parseArgs(['0.3.0', '--dry-run'], ['--dry-run']).dryRun, true);
  assert.equal(parseArgs(['0.3.0', '--allow-unpushed'], ['--allow-unpushed']).allowUnpushed, true);
  assert.equal(parseArgs(['0.3.0', '--check-files', '--ci'], ['--check-files', '--ci']).checkFiles, true);
  for (const [argv, message] of [
    [['0.3.0', '--changelog'], '--changelog needs a file'],
    [['0.3.0', '--changelog', '--allow-jump'], '--changelog needs a file'],
    [['0.3.0', '--changelog='], '--changelog needs a file'],
    [['0.3.0', '--bogus'], 'unknown option --bogus'],
    [['0.3.0', '--dry-run'], 'unknown option --dry-run'],
    [['0.3.0', '--allow-jump=yes'], 'takes no value'],
    [['0.3.0', '0.4.0'], 'unexpected argument'],
  ]) {
    assert.throws(() => parseArgs(argv), (e) => e instanceof UsageError && e.message.includes(message), argv.join(' '));
  }
});

// The five files, small.
const fixture = (v, { lockCrates = ['kova', 'kova-core', 'kova-engine', 'kova-service'], rustFirst = false } = {}) => ({
  'package.json': JSON.stringify({ name: 'kova', version: v, dependencies: { svelte: '^5.0.0' } }, null, 2) + '\n',
  'package-lock.json': JSON.stringify({ name: 'kova', version: v, lockfileVersion: 3, packages: { '': { name: 'kova', version: v }, 'node_modules/svelte': { version: '5.0.0' } } }, null, 2) + '\n',
  'src-tauri/tauri.conf.json': JSON.stringify({ productName: 'Kova', version: v }, null, 2) + '\n',
  'Cargo.toml': `[workspace]\nmembers = ["a"]\n\n[workspace.package]\n${rustFirst ? 'rust-version = "1.90"\n' : ''}version = "${v}"\nrust-version = "1.90"\nedition = "2021"\n\n[workspace.dependencies]\nx = "1"\n`,
  'Cargo.lock': [...lockCrates.map((n) => `[[package]]\nname = "${n}"\nversion = "${v}"\n`), '[[package]]\nname = "serde"\nversion = "1.0.0"\n'].join('\n'),
});

test('applyVersion: all five files, or an error naming one', () => {
  for (const eol of [(s) => s, crlf]) {
    const files = Object.fromEntries(Object.entries(fixture('0.2.10')).map(([k, s]) => [k, eol(s)]));
    const out = applyVersion(files, '0.3.0');
    assert.deepEqual(verifyFiles('0.3.0', out), []);
    assert.ok(out['Cargo.lock'].includes('name = "serde"' + (eol === crlf ? '\r\n' : '\n') + 'version = "1.0.0"'));
    assert.ok(out['Cargo.toml'].includes('rust-version = "1.90"'));
    assert.equal(out['Cargo.lock'].includes('0.2.10'), false);
    assert.equal(out['package-lock.json'].includes('"0.2.10"'), false);
    assert.ok(out['package.json'].includes('"svelte": "^5.0.0"'));
  }
  const first = applyVersion(fixture('0.2.10', { rustFirst: true }), '0.3.0');
  assert.ok(first['Cargo.toml'].includes('rust-version = "1.90"\nversion = "0.3.0"'));
  assert.throws(() => applyVersion(fixture('0.2.10', { lockCrates: ['kova', 'kova-core', 'kova-engine'] }), '0.3.0'), /^Error: Cargo\.lock: expected the crates/);
  assert.throws(() => applyVersion(fixture('0.2.10', { lockCrates: ['kova', 'kova', 'kova-core', 'kova-engine', 'kova-service'] }), '0.3.0'), /Cargo\.lock/);
  const noRoot = fixture('0.2.10');
  noRoot['package-lock.json'] = '{"lockfileVersion": 3}';
  assert.throws(() => applyVersion(noRoot, '0.3.0'), /package-lock\.json: expected 2 root version entries, found 0/);
  const noToml = fixture('0.2.10');
  noToml['Cargo.toml'] = '[workspace.package]\nrust-version = "1.90"\n';
  assert.throws(() => applyVersion(noToml, '0.3.0'), /Cargo\.toml: no version found/);
  assert.throws(() => applyVersion({ ...fixture('0.2.10'), 'package.json': '{}' }, '0.3.0'), /package\.json: no version found/);
  assert.throws(() => applyVersion({ 'package.json': '{}' }, '0.3.0'), /missing/);
});

test('checkFiles reads the real files of the repository', () => {
  const current = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;
  assert.deepEqual(checkFiles(current), []);
  const bad = checkFiles('9.9.9');
  assert.ok(bad.length >= 5);
  for (const file of ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json', 'Cargo.toml', 'Cargo.lock']) {
    assert.ok(bad.some((e) => e.includes(file) && e.includes('not 9.9.9')), file);
  }
});

test('the cores line', () => {
  const pins = { mihomo: { tag: 'v1.19.34' }, xray: { tag: 'v26.11.9' }, shoes: { tag: 'v0.3.7' } };
  assert.equal(coresLine(pins), 'Ships Mihomo v1.19.34, Xray v26.11.9 and Shoes v0.3.7.');
  assert.equal(coresLine({ mihomo: pins.mihomo, xray: pins.xray }), 'Ships Mihomo v1.19.34 and Xray v26.11.9.');
  assert.throws(() => coresLine({}));
});
