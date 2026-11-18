// Checks a version number against the versioning policy (docs/building.md, "Versioning
// and releases"):
//   node scripts/check-version.mjs <version> [--changelog <file>] [--allow-jump]
//                                  [--allow-no-tags] [--check-files] [--ci] [--json]
// <version> is X.Y.Z or X.Y.Z-beta.N. The check reads the tags of the repository; the
// functions below are pure, so the tests need no git.
//   --changelog   the release notes decide the least bump the version must be
//   --allow-jump  lets a deliberate jump through (1.0.0); a number not above the latest
//                 stable version is an error even then
//   --allow-no-tags  a repository without any stable tag is not an error
//   --check-files the five files that record the version must all carry it
//   --ci          for the build of a tag that exists already (release.yml)
//   --json        print the result as JSON

import { execFileSync } from 'node:child_process';
import { readFileSync, realpathSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-beta\.([1-9]\d*))?$/;
const RANK = { patch: 1, minor: 2, major: 3 };

/** `0.2.10` or `0.2.10-beta.1` as numbers, or null for anything else. */
export function parse(version) {
  const m = VERSION.exec(typeof version === 'string' ? version : '');
  if (!m) return null;
  const [major, minor, patch] = [m[1], m[2], m[3]].map(Number);
  const beta = m[4] === undefined ? null : Number(m[4]);
  const base = `${major}.${minor}.${patch}`;
  return { major, minor, patch, beta, base, text: beta === null ? base : `${base}-beta.${beta}` };
}

/** Negative, zero or positive: the order of the numbers, never of the text. */
const compare = (a, b) => a.major - b.major || a.minor - b.minor || a.patch - b.patch;

/** The stable versions among git tags (`v0.2.10`); betas and anything else are left out. */
export function stableTags(tagList) {
  const out = [];
  for (const tag of tagList ?? []) {
    if (typeof tag !== 'string' || !tag.startsWith('v')) continue;
    const v = parse(tag.slice(1));
    if (v && v.beta === null) out.push(v);
  }
  return out;
}

/** The highest of parsed versions, or null. */
export function highest(versions) {
  let top = null;
  for (const v of versions) if (!top || compare(v, top) > 0) top = v;
  return top;
}

/** The versions that may follow H: next patch, next minor, next major. */
export function nextSteps(h) {
  return [`${h.major}.${h.minor}.${h.patch + 1}`, `${h.major}.${h.minor + 1}.0`, `${h.major + 1}.0.0`];
}

/** Android's versionCode as Tauri computes it (a pre-release suffix is ignored). */
export const versionCode = (v) => {
  const p = typeof v === 'string' ? parse(v) : v;
  return p.major * 1_000_000 + p.minor * 1_000 + p.patch;
};

/** Which part of H the version moves: patch, minor or major. */
function bumpOf(v, h) {
  if (v.major !== h.major) return 'major';
  if (v.minor !== h.minor) return 'minor';
  return 'patch';
}

// What each `###` heading of the notes calls for.
const SECTIONS = {
  added: 'minor', changed: 'minor', removed: 'minor', deprecated: 'minor',
  breaking: 'breaking', 'breaking changes': 'breaking',
  fixed: 'patch', security: 'patch',
};

/**
 * Reads the release notes. The least bump they call for: minor for Added, Changed,
 * Removed, Deprecated or Breaking, patch for Fixed or Security, null without a bullet.
 * `major` is the major version the release comes from; from 1 on a Breaking bullet calls
 * for a major. Also returns `heading` (the first `##` line, or null), `errors` (a section
 * with bullets and a name that is not known) and `counts`.
 */
export function requiredBump(changelogText, { major = 0 } = {}) {
  // Comments and horizontal rules are no entries; CRLF and tabs are fine.
  const text = String(changelogText ?? '').replace(/<!--[\s\S]*?-->/g, '');
  const counts = { minor: 0, breaking: 0, patch: 0 };
  const unknown = new Set();
  let heading = null;
  let section;
  for (const line of text.split(/\r?\n/)) {
    if (/^\s*([*_-])(\s*\1){2,}\s*$/.test(line)) continue;
    const h = /^(#{2,3})[ \t]+(.*?)[ \t]*$/.exec(line);
    if (h) {
      if (h[1].length === 2) {
        if (heading !== null) break;
        heading = h[2];
        section = undefined;
      } else {
        section = h[2].trim();
      }
      continue;
    }
    if (section === undefined || !/^[ \t]*[*-][ \t]+\S/.test(line)) continue;
    const kind = SECTIONS[section.toLowerCase()];
    if (kind) counts[kind]++;
    else unknown.add(section);
  }
  let bump = null;
  if (counts.minor + counts.breaking > 0) bump = 'minor';
  else if (counts.patch > 0) bump = 'patch';
  if (counts.breaking > 0 && major >= 1) bump = 'major';
  const errors = [...unknown].map(
    (s) => `unknown section ${s}: use Added, Changed, Removed, Deprecated, Breaking, Fixed or Security`,
  );
  return { bump, counts, breaking: counts.breaking > 0, heading, errors };
}

/**
 * Checks a version against the policy. `tags` is the list of git tags. Returns
 * `{ ok, errors, warnings, info }`.
 */
export function check({ version, tags = [], changelog, allowJump = false, allowNoTags = false, ci = false }) {
  const errors = [];
  const warnings = [];
  const info = {
    latest: null, bump: null, required: null, versionCode: null, previousCode: null,
    beta: false, betaBase: null, betaNumber: null, lastBeta: null,
  };
  const done = () => ({ ok: errors.length === 0, errors, warnings, info });
  const v = parse(version);
  if (!v) {
    errors.push(`"${version}" is not a valid version: use X.Y.Z or X.Y.Z-beta.N (N from 1)`);
    return done();
  }
  const tag = `v${v.text}`;
  // In CI the tag being built exists already, and must not count against itself.
  const others = ci ? tags.filter((t) => t !== tag) : tags;
  if (!ci && tags.includes(tag)) errors.push(`the tag ${tag} exists already`);

  info.beta = v.beta !== null;
  info.versionCode = versionCode(v);
  if (v.major > 2000) errors.push(`the major version ${v.major} is above 2000 (the versionCode would not fit)`);
  if (v.minor > 999) errors.push(`the minor version ${v.minor} is above 999 (the versionCode would overlap)`);
  if (v.patch > 999) errors.push(`the patch version ${v.patch} is above 999 (the versionCode would overlap)`);

  const h = highest(stableTags(others));
  if (h) {
    info.latest = h.base;
    info.previousCode = versionCode(h);
  } else if (ci || allowNoTags) {
    warnings.push('no stable tag found: nothing to compare the version with');
  } else {
    errors.push('no stable tag found: fetch the tags (git fetch --tags), or pass --allow-no-tags');
  }

  // A beta is only for a minor or major release candidate.
  const betas = [];
  for (const t of others) {
    const p = t.startsWith('v') ? parse(t.slice(1)) : null;
    if (p && p.beta !== null && p.base === v.base) betas.push(p.beta);
  }
  if (v.beta !== null) {
    info.betaBase = v.base;
    info.betaNumber = v.beta;
    if (v.patch !== 0) {
      errors.push(`a beta is only for an X.Y.0 release, not for the patch release ${v.base}`);
    }
    if (ci ? v.beta > 1 && !betas.includes(v.beta - 1) : v.beta !== Math.max(0, ...betas) + 1) {
      const next = Math.max(0, ...betas) + 1;
      errors.push(
        betas.includes(v.beta)
          ? `beta.${v.beta} of ${v.base} exists already (the next is beta.${next})`
          : `beta.${v.beta} of ${v.base} skips a number (${betas.length ? `the next is beta.${next}` : 'the first is beta.1'})`,
      );
    }
  } else if (v.patch === 0 && betas.length) {
    info.lastBeta = `${v.base}-beta.${Math.max(...betas)}`;
  }

  if (h) {
    if (compare(v, h) <= 0) {
      errors.push(`${v.base} is not above the latest stable ${h.base}: a version never goes down or repeats`);
    } else {
      info.bump = bumpOf(v, h);
      // The step to 1.0.0 is the one that waits for the 1.0.0 checklist, so it is a jump too.
      const steps = nextSteps(h).filter((s) => h.major > 0 || !s.startsWith('1.'));
      if (!ci && !steps.includes(v.base)) {
        const msg = `${v.base} is not one step after the latest stable ${h.base} (expected ${steps.join(', ')})`;
        if (allowJump) warnings.push(`${msg}: allowed by --allow-jump`);
        else errors.push(`${msg}; --allow-jump allows a deliberate jump`);
      }
      if (versionCode(v) <= versionCode(h)) {
        errors.push(`the Android versionCode ${versionCode(v)} is not above ${versionCode(h)} of ${h.base}`);
      }
    }
  }

  if (changelog !== undefined && changelog !== null) {
    const need = requiredBump(changelog, { major: h ? h.major : 0 });
    info.required = need.bump;
    errors.push(...need.errors);
    if (need.heading === null) {
      warnings.push('the changelog has no "## Kova X.Y.Z" heading, so its version is not checked');
    } else if (!new RegExp(`(?<![\\d.])${v.base.replaceAll('.', '\\.')}(?!\\d)`).test(need.heading)) {
      errors.push(`the changelog heading "${need.heading}" does not name ${v.base}`);
    }
    if (!need.bump) {
      if (!need.errors.length) errors.push('the changelog has no entries (bullets under Added, Changed, Fixed or Security)');
    } else if (info.bump) {
      const label = need.bump === 'major' ? 'Breaking' : need.bump === 'minor' ? 'Added/Changed' : 'Fixed/Security';
      if (RANK[info.bump] < RANK[need.bump]) {
        errors.push(`the changelog has ${label} entries, so this release must be a ${need.bump} (it is a ${info.bump})`);
      } else if (RANK[info.bump] > RANK[need.bump]) {
        warnings.push(`the changelog only calls for a ${need.bump}, and this release is a ${info.bump}`);
      }
    }
  }
  return done();
}

/** The five files that record the version. */
export const VERSION_FILES = ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json', 'Cargo.toml', 'Cargo.lock'];
const OWN_CRATES = ['kova', 'kova-core', 'kova-engine', 'kova-service'];
// The patterns bump-version.mjs rewrites. Cargo.toml: the line `version = ` of the
// [workspace.package] table (a `rust-version = ` line cannot match).
const CARGO_TOML = /(\[workspace\.package\][^[]*?^version = )"([^"]+)"/m;
const CARGO_LOCK = /(name = "(kova(?:-core|-engine|-service)?)"\r?\nversion = )"([^"]+)"/g;
const LOCK_ROOT = /("name": "kova",\s*"version": )"([^"]+)"/g;
const FIRST_VERSION = /"version": "[^"]+"/;

/** Errors for each file in `files` ({ path: text }) that does not carry exactly the version. */
export function verifyFiles(version, files) {
  const errors = [];
  const expect = (file, found) => {
    if (found !== version) errors.push(`${file} says ${found ?? 'no version'}, not ${version}`);
  };
  const guard = (file, fn) => {
    try {
      fn();
    } catch (e) {
      errors.push(`${file}: ${e.message}`);
    }
  };
  guard('package.json', () => expect('package.json', JSON.parse(files['package.json']).version));
  guard('package-lock.json', () => {
    const lock = JSON.parse(files['package-lock.json']);
    expect('package-lock.json (version)', lock.version);
    expect('package-lock.json (packages[""])', lock.packages?.['']?.version);
  });
  guard('src-tauri/tauri.conf.json', () => expect('src-tauri/tauri.conf.json', JSON.parse(files['src-tauri/tauri.conf.json']).version));
  guard('Cargo.toml', () => expect('Cargo.toml', CARGO_TOML.exec(files['Cargo.toml'])?.[2]));
  guard('Cargo.lock', () => {
    const found = [...files['Cargo.lock'].matchAll(CARGO_LOCK)];
    for (const name of OWN_CRATES) expect(`Cargo.lock (${name})`, found.find((f) => f[2] === name)?.[3]);
  });
  return errors;
}

/**
 * The new text of the five files ({ path: text }) with the version set, or an error
 * naming the file when any of them does not have what it should. Nothing is written
 * here, so a failure leaves every file as it was.
 */
export function applyVersion(files, version) {
  const out = {};
  const fail = (file, why) => {
    throw new Error(`${file}: ${why}`);
  };
  for (const file of VERSION_FILES) if (typeof files[file] !== 'string') fail(file, 'missing');
  const first = (file, pattern, replacement) => {
    if (!pattern.test(files[file])) fail(file, 'no version found');
    return files[file].replace(pattern, replacement);
  };
  out['package.json'] = first('package.json', FIRST_VERSION, `"version": "${version}"`);
  // The lockfile repeats the root version twice; `npm ci` wants them in step.
  const roots = [...files['package-lock.json'].matchAll(LOCK_ROOT)];
  if (roots.length !== 2) fail('package-lock.json', `expected 2 root version entries, found ${roots.length}`);
  out['package-lock.json'] = files['package-lock.json'].replace(LOCK_ROOT, `$1"${version}"`);
  out['src-tauri/tauri.conf.json'] = first('src-tauri/tauri.conf.json', FIRST_VERSION, `"version": "${version}"`);
  out['Cargo.toml'] = first('Cargo.toml', CARGO_TOML, `$1"${version}"`);
  const crates = [...files['Cargo.lock'].matchAll(CARGO_LOCK)].map((m) => m[2]).sort();
  if (crates.join() !== [...OWN_CRATES].sort().join()) {
    fail('Cargo.lock', `expected the crates ${OWN_CRATES.join(', ')} once each, found ${crates.join(', ') || 'none'}`);
  }
  out['Cargo.lock'] = files['Cargo.lock'].replace(CARGO_LOCK, `$1"${version}"`);
  // What was written must read back as the version.
  const bad = verifyFiles(version, out);
  if (bad.length) throw new Error(bad.join('; '));
  return out;
}

/** Errors for each of the five files that does not carry exactly the version. */
export function checkFiles(version, dir = root) {
  const files = {};
  const errors = [];
  for (const file of VERSION_FILES) {
    try {
      files[file] = readFileSync(join(dir, file), 'utf8');
    } catch (e) {
      errors.push(`${file}: ${e.message}`);
    }
  }
  if (errors.length) return errors;
  return verifyFiles(version, files);
}

/** The tags of this repository. */
export function readTags() {
  const out = execFileSync('git', ['tag', '--list', 'v*'], { cwd: root, encoding: 'utf8' });
  return out.split(/\r?\n/).map((t) => t.trim()).filter(Boolean);
}

/** A mistake in the command line. */
export class UsageError extends Error {}

const camel = (flag) => flag.slice(2).replace(/-(\w)/g, (_, c) => c.toUpperCase());

/**
 * The options of the command line, shared by check-version, bump-version and release.
 * `extra` lists the other flags (without a value) the caller allows. Both `--flag value`
 * and `--flag=value` work for --changelog; anything unknown throws a UsageError.
 */
export function parseArgs(argv, extra = []) {
  const bools = ['--allow-jump', '--allow-no-tags', ...extra];
  const opts = { version: undefined, changelog: undefined };
  for (const flag of bools) opts[camel(flag)] = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith('--')) {
      if (opts.version !== undefined) throw new UsageError(`unexpected argument ${a}`);
      opts.version = a;
      continue;
    }
    const eq = a.indexOf('=');
    const name = eq < 0 ? a : a.slice(0, eq);
    if (name === '--changelog') {
      const value = eq >= 0 ? a.slice(eq + 1) : argv[i + 1] !== undefined && !argv[i + 1].startsWith('--') ? argv[++i] : '';
      if (!value) throw new UsageError('--changelog needs a file');
      opts.changelog = value;
    } else if (bools.includes(name)) {
      if (eq >= 0) throw new UsageError(`${name} takes no value`);
      opts[camel(name)] = true;
    } else {
      throw new UsageError(`unknown option ${name}`);
    }
  }
  return opts;
}

/** Runs the check on this repository. Returns the result. */
export function runCheck(opts) {
  let changelog;
  if (opts.changelog !== undefined) {
    try {
      changelog = readFileSync(resolve(opts.changelog), 'utf8');
    } catch (e) {
      return { ok: false, errors: [`cannot read the changelog ${opts.changelog}: ${e.message}`], warnings: [], info: {} };
    }
  }
  const result = check({
    version: opts.version, tags: readTags(), changelog, allowJump: opts.allowJump, allowNoTags: opts.allowNoTags, ci: opts.ci,
  });
  if (opts.checkFiles && parse(opts.version)) {
    const bad = checkFiles(opts.version);
    result.errors.push(...bad);
    result.ok = result.errors.length === 0;
  }
  return result;
}

function summary(version, r) {
  const i = r.info;
  if (!i.versionCode) return null;
  const kind = i.beta ? `${i.bump ? `${i.bump}, ` : ''}beta ${i.betaNumber} of ${i.betaBase}` : i.bump;
  const code = i.previousCode ? `versionCode ${i.previousCode} -> ${i.versionCode}` : `versionCode ${i.versionCode}`;
  return `${i.latest ? `latest stable ${i.latest} -> ` : ''}new ${version}${kind ? ` (${kind})` : ''}, ${code}`;
}

export function printResult(version, r, { ci = false } = {}) {
  const line = summary(version, r);
  if (line) console.log(line);
  for (const e of r.errors) console.error(`error: ${e}`);
  for (const w of r.warnings) console.warn(`warning: ${w}`);
  if (r.ok) console.log('version ok');
  // A final X.Y.0 must hold the code of its last beta and little else.
  if (r.info.lastBeta && !ci) {
    console.log(`\n${version} follows v${r.info.lastBeta}; what changed since then:`);
    try {
      execFileSync('git', ['diff', '--stat', `v${r.info.lastBeta}..HEAD`], { cwd: root, stdio: 'inherit' });
    } catch {
      console.log('(could not read the diff)');
    }
    console.log('Reminder: besides the version, only translation and documentation fixes may differ from the last beta.');
  }
}

const isMain = () => {
  try {
    return realpathSync(fileURLToPath(import.meta.url)) === realpathSync(resolve(process.argv[1]));
  } catch {
    return false;
  }
};

if (isMain()) {
  const usage = 'usage: node scripts/check-version.mjs <version> [--changelog <file>] [--allow-jump] [--allow-no-tags] [--check-files] [--ci] [--json]';
  let opts;
  try {
    opts = parseArgs(process.argv.slice(2), ['--check-files', '--ci', '--json']);
  } catch (e) {
    if (!(e instanceof UsageError)) throw e;
    console.error(`check-version: ${e.message}\n${usage}`);
    process.exit(1);
  }
  if (!opts.version) {
    console.error(usage);
    process.exit(1);
  }
  const result = runCheck(opts);
  if (opts.json) console.log(JSON.stringify(result, null, 2));
  else printResult(opts.version, result, opts);
  process.exit(result.ok ? 0 : 1);
}
