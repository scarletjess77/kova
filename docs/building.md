# Building and testing

## Desktop

Requirements: Rust 1.90 or newer, Node.js 20+ (22+ for `npm run test:scripts`), and on Windows the MSVC C++ build tools.

```bash
npm install
npm run tauri dev        # run with hot reload
npm run tauri build      # release build and installer
```

Set `KOVA_DATA_DIR` to keep development data apart from a real profile (`dev-data/` is ignored by git). On first launch Kova downloads any missing core, at the pinned version.

## Android

Requirements: Android SDK with platform 36 and NDK r28, JDK 17 or 21, Go, and the Rust targets `aarch64-linux-android` (and `armv7-linux-androideabi` for 32-bit devices). With `ANDROID_HOME`, `NDK_HOME` and `JAVA_HOME` set:

```bash
npm run android:build                      # release APK, arm64 (unsigned unless KOVA_ANDROID_KEYSTORE is set)
npm run android:cores && npx tauri android build --apk --target aarch64 --debug   # debug APK, installable as is
node scripts/android-cores.mjs armeabi-v7a && npx tauri android build --apk --target armv7   # 32-bit
```

`scripts/android-cores.mjs` gets the cores pinned in `android-cores.json`: Xray's official Android build (checked by SHA-256) for arm64, and Mihomo built from its official source with Go (and Xray too for 32-bit, which has no official Android build). The APK is in `src-tauri/gen/android/app/build/outputs/apk/`.

## Tests

```bash
cargo test                                                              # unit tests
cargo test -p kova-engine --test e2e -- --ignored --nocapture           # real cores, local server
cargo test -p kova-engine --test geodata -- --ignored --nocapture       # downloads routing data
cargo test -p kova-service --test service_e2e -- --ignored --nocapture  # service path (run e2e first)
npm run check                                                           # UI types and translations
npm run test:scripts                                                    # the version scripts
```

The end-to-end test downloads the real cores, starts a local Xray server with VLESS over WebSocket and XHTTP, imports both as share links, tests latency, connects, and fetches a page through each node, including the bridged XHTTP one. It also checks switching away from a dead node and connecting without routing data. None of the tests touch the system proxy or enable TUN.

`npm run check` also checks that every interface text has a Russian, Persian and Chinese translation (`ui/src/lib/lang/`; the English text is the key), and that none is left over.

## Releases

The cores every build ships, desktop installers included, are the versions pinned in `scripts/android-cores.json`; the end-to-end tests run against the same. To move to a new Mihomo, Xray or Shoes, change the pin (the **Core versions** workflow opens an issue when one is out), run `node scripts/pin-cores.mjs` to write the new files' SHA-256 beside it, and run the end-to-end tests. A pinned core is installed only when the download has the pinned checksum, so a release whose files were replaced later is refused.

A `v<version>` tag starts the **Release** workflow, which builds and signs every installer and APK into a draft release. Installed copies see the update once the draft is published. The **End to end** workflow runs the end-to-end tests every night, whenever the pinned cores change, and when started by hand. The **CI** workflow runs the tests on Windows and Linux for every push to `main`, `master` and `beta` and for every pull request; macOS runs when the workflow is started by hand (Actions → CI → Run workflow), as hosted Macs are often unavailable.

## Versioning and releases

A version is `X.Y.Z` (stable) or `X.Y.Z-beta.N` (a beta, N from 1). Any other suffix is refused. What the numbers mean, and which changelog sections decide it:

| Bump | When | Changelog |
| --- | --- | --- |
| Patch | Only fixes, security fixes, translations, and core bumps with no visible change. | Only `Fixed` or `Security` entries. |
| Minor | Anything a user can notice. Before 1.0, breaking changes too, with a "Breaking" note. | At least one `Added` or `Changed` entry (or `Breaking`, before 1.0). |
| Major | From 1.0 on: a breaking change to the settings, profile or backup formats, or dropping a platform. | A non-empty `Breaking` section (from 1.0 on). |

The new stable version must be exactly one step after the highest stable tag H: `H.patch + 1`, or `H.minor + 1` with patch 0, or `H.major + 1` with minor and patch 0. So 0.2.10 is followed by 0.2.11 or 0.3.0, and not by 0.2.100, 0.3.1 or 0.30.0. The step from 0.x to 1.0.0 waits for the checklist below, so it counts as a jump too. A number not above H is always refused. `--allow-jump` lets a deliberate jump through (the 1.0.0 release), nothing else.

### Betas

A beta is for a minor or major release candidate only, so its base ends in `.0`: `0.3.0-beta.1`, never `0.2.11-beta.1`. There are no betas for patches. The base must be a valid next step after H, and N is 1 for the first beta of a base and the previous N + 1 afterwards, with no gaps and no repeats. A beta is tagged on `main` like any release. It is published as a GitHub pre-release: never "latest", never sent to winget, and installed copies see it only with the beta channel setting on. The final `X.Y.0` must contain the same code as the last beta, plus the version, translation and documentation fixes only. The script cannot check that, so it prints `git diff --stat v<last beta>..HEAD` and a reminder when a beta of the same base exists.

### Android versionCode

Tauri computes it as `major * 1_000_000 + minor * 1_000 + patch` (0.2.5 gives 2005; a beta suffix is ignored, so a beta and its final have the same code). It must only ever increase, so minor and patch must stay below 1000 and major at or below 2000, and the guard refuses a version whose code is not above the latest stable one.

### Why a lower number must never come later

GitHub's "latest" release is the newest by publish date, not by number. A lower version published after a higher one becomes "latest" and installed copies would be offered a downgrade. The number must never go down or repeat, whatever the order of publishing.

### The guards

- `scripts/check-version.mjs` checks all of the above against the tags of the repository (and the changelog, with `--changelog`). `bump-version.mjs` and `release.mjs` run it first.
- The **Release** workflow runs `check-version.mjs --check-files --ci` on a tag: the five files that carry the version (`package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `Cargo.toml` and `Cargo.lock`) must say exactly the version of the tag, and the tag must be above the latest stable one. The one-step rule is skipped there, because the tag exists already. A consequence by design: once a newer stable tag exists, running the build of an older tag again fails, as that version is no longer above the latest one. Such a build cannot be re-run; cut the next release instead.
- The **CI** workflow runs the script tests (`npm run test:scripts`).

With `--changelog <file>` (the release notes in the style of the changelog: `## Kova X.Y.Z`, then `### Added`, `### Changed`, `### Fixed`, `### Security`, optionally `### Breaking`) the least bump follows from the entries: minor for `Added`, `Changed` or `Breaking`, else patch for `Fixed` or `Security`, else an error. A smaller bump than that is an error; a bigger one a warning.

### Making a release

Fetch the tags first, then, on `main`:

```bash
npm run version:check -- 0.3.0 --changelog notes.md    # only checks
npm run release -- 0.3.0 --changelog notes.md          # checks, sets the version, cargo check, commit, signed tag
git tag -v v0.3.0                                      # the release script runs it; look at the result
git push origin main v0.3.0                            # starts the Release workflow (a draft release)
```

`npm run release -- <version> --dry-run` prints the commands without running them. `release.mjs` fetches `origin main` and the tags, and needs `main` with a clean tree, not behind `origin/main`, and not ahead of it unless `--allow-unpushed` is given (the unpushed commits are listed). It never pushes. The commit is "Set the version to 0.3.0" and the tag is signed with `git tag -s -a v0.3.0 -m "v0.3.0"`, so signing must be configured (`user.signingkey`, and `gpg.format` for an SSH key); the script refuses without a signing key, before it changes anything. If a step fails after the version was set, it prints how to undo it. A repository without any stable tag is refused unless `--allow-no-tags` is given. `--dry-run` fetches nothing and exits 1 when a real run would refuse. `node scripts/bump-version.mjs <version>` alone sets the five files (same checks, `--force` skips them with a warning); it works out all five first and writes nothing unless every file has what it should.

Under the changelog in the release notes, paste the line `node scripts/cores-line.mjs` prints, for example `Ships Mihomo v1.19.34, Xray v26.11.9 and Shoes v0.3.7.`.

### Version 1.0.0

1.0.0 needs all of these:

- The settings, profile and backup formats are frozen, with a documented schema version and a tested migration.
- The nightly end-to-end run has been green for 14 days in a row.
- Each platform has been tested by a person on real hardware for one full release cycle: Windows with the service, Android phone and TV, Linux, and macOS.
- No open high-severity security finding.
- 30 days without a breaking settings change.

It is released with `--allow-jump`.
