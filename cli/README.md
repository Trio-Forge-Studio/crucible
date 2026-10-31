# Crucible CLI

`crucible` is the command line for Crucible games. `npx crucible docs cli` lists every command. This file covers building and testing the CLI. The guides cover using it: `docs/cli.md` for the commands and `docs/oc.md` for Open Cloud keys, operations, flows, and exit codes.

## Docs topics

Each file in `docs` is one topic, named by its file stem. `build.rs` embeds every `docs/*.md` file in the binary and reruns when the folder changes. `crucible docs` lists the topics, and `crucible docs <topic>` prints one.

A topic starts with a `# Title` line, a blank line, and a one-line summary that ends with a period. `crucible docs` prints that summary, and `cargo test` fails for a topic without one. Use plain Markdown, because a website will render the same files. Document only what the code exports today. Keep the `## crucible <command>` sections in `docs/cli.md` in alphabetical order.

## Build the CLI

Run this from `cli/` with the toolchain pinned in the repository's `rust-toolchain.toml`. The binary is `target/release/crucible`.

```console
cargo build --release --locked --package crucible
```

## npm packages

The CLI ships on npm the way oxlint does. The `crucible` package holds a Node shim and the `crucible` bin. Each supported platform has its own package with the binary: `@trioforge/crucible-cli-win32-x64`, `@trioforge/crucible-cli-linux-x64`, and `@trioforge/crucible-cli-darwin-arm64`. The shim lists them as optional dependencies, so npm and pnpm install only the one that matches the machine's `os` and `cpu`. The shim runs that binary with the same arguments and exits with its code. If the platform package is missing, the shim exits 1 and names the package.

`npm/stage.mjs` writes the packages from built binaries. Every package takes its version from `[workspace.package]` in `Cargo.toml`. The runtime, CLI, and npm packages use the root MIT license. The Cargo crates remain unpublished on crates.io.

```console
node npm/stage.mjs build/npm win32-x64=target/release/crucible.exe
```

`npm/package.test.mjs` stages the debug binary from `cargo test`, packs the packages with `npm pack`, installs them into a temporary project, and runs `npx crucible`. Set `CRUCIBLE_BINARY` to test another binary.

## Release packages

`.github/workflows/release.yml` runs on pull requests and `v*` tag pushes. It builds MSVC Windows x64, static musl Linux x64, and native Apple Silicon macOS binaries on Blacksmith. The Mac job uses `blacksmith-6vcpu-macos-15`, documented in [Blacksmith runner types](https://docs.blacksmith.sh/blacksmith-runners/overview). It ad-hoc signs the Mach-O binary, verifies its signature and architecture, and executes it through the npm shim on a real Mac. Ad-hoc signing does not provide Apple notarization or a Developer ID identity. Linux checks `file` and `ldd` and executes the static binary.

The packaging job builds the runtime and transformer, generates `THIRD_PARTY_NOTICES` with cargo-about from the locked dependency graph, and stages all five packages at the version in `cli/Cargo.toml`. The platform packages include the notices and MIT license. Notices include dependency license texts and the Rojo source location for its MPL-2.0 license when Rojo is bundled. New dependency licenses outside `cli/about.toml` fail the job for review.

`node cli/npm/release.mjs <binaries> <out> <notices>` expects directories named `linux-x64`, `win32-x64`, and `darwin-arm64`. It checks `npm pack --dry-run` for runtime output, transformer, and side entries, then packs five tarballs. CI runs `npm publish --dry-run` on each tarball without credentials. Tags must match `v` plus the Cargo version.

The publish job runs on GitHub-hosted `ubuntu-latest`. npm provenance rejects self-hosted runners, including Blacksmith. Build jobs stay on Blacksmith. The publish job uses the `npm-release` GitHub Environment and its `NPM_TOKEN` secret. Environment protection gates access. A repository secret would also be available to other authorized workflows that reference it, so the event condition alone is not a secret access boundary.

### Prepare the first release

1. Confirm with npm that the operator's account can claim the unscoped `crucible` name. npm's security team held that name until 2025-11-20. The elapsed date does not prove claim rights. The account also needs publish access to the `@trioforge` organization and all four scoped packages.
2. Make the source repository public before publishing. [npm provenance](https://docs.npmjs.com/generating-provenance-statements/) requires public source and a supported hosted runner. Keep the publish job on GitHub-hosted Ubuntu even if builds use Blacksmith.
3. Create the `npm-release` GitHub Environment. Limit its deployment tags to `v*`, require reviewers, and restrict who can approve releases. Store `NPM_TOKEN` there, not as a repository secret. Use an expiring granular token with package read/write permission for unscoped `crucible` and the four `@trioforge` packages. For new names, include permission to create the packages in those scopes. Enable bypass 2FA for noninteractive publication where npm account policy permits it. An organization permission alone does not grant package publication.
4. This workflow keeps token authentication for the first publication because the package names must exist before their [trusted publisher](https://docs.npmjs.com/trusted-publishers/) records can be configured. After claiming all five names, prefer OIDC. Configure each package's GitHub publisher for `Trio-Forge-Studio/crucible`, `release.yml`, and environment `npm-release`. Then remove the token requirement and `NODE_AUTH_TOKEN` from the workflow and revoke the token. Keep `id-token: write` and use npm 11.5.1 or newer. These setup actions belong to the operator.

### Cut a release

1. Bump `[workspace.package].version` in `cli/Cargo.toml` and `version` in `packages/crucible/package.json` together. Run Cargo to refresh `cli/Cargo.lock` and retain it. Generated npm packages inherit the Cargo version.
2. Choose a version never used by any of the five packages. The registry preflight checks both `versions` and `time` keys before publication. Known burned versions of `crucible` are `0.0.0`, `0.0.1`, `0.3.0`, `0.3.1`, `0.3.2`, `0.3.3`, `0.3.4`, `0.3.5`, `0.4.0`, `0.4.1`, and `9.0.0`. Unpublishing does not make a version reusable. The local denylist also catches these versions if registry history disappears.
3. Run the repository checks and review the five package dry runs in CI. Commit and land the version change before tagging.
4. Create and push the exact version tag, replacing `X.Y.Z` below.

```console
git tag vX.Y.Z && git push origin refs/tags/vX.Y.Z
```

The tag must match the Cargo version. Approve the protected environment deployment after inspecting build, notices, preflight, and dry-run logs. Prerelease versions require `npm publish --tag next` or another non-`latest` dist-tag. This workflow selects `next` for versions containing a hyphen and `latest` for stable versions.

### Recover and verify

npm cannot publish five packages atomically. After a partial publication, rerun **failed jobs only** in the same Actions run. The successful packaging job's unused-version preflight and tarballs stay intact. The publish job reads each tarball's name and version, skips already-published tarballs only when registry integrity matches, and publishes missing platform packages and runtime before the shim. A full workflow rerun repeats preflight and rejects the partly used version. If tarballs differ or the artifacts have expired, investigate and release a fresh shared version. Never delete and recreate a tag or try to reuse an unpublished version.

After release, run `npm view <name>@X.Y.Z version dist.integrity dist.attestations` for all five names. Check the provenance badge and linked source on each npm package page. On Windows x64, Linux x64, and macOS arm64, run `npx crucible@X.Y.Z --version` and confirm the shared version and embedded Rojo version.

## Create a game


`npx crucible create my-game [path]` writes the embedded game template and installs dependencies. `--no-install` skips install. See `docs/cli.md` for arguments and exit codes.

In the generated game, run `npx crucible dev` to compile on save and sync the game to Studio.

`crates/crucible/template` holds the real template files. `src/create.rs` embeds an explicit file table. Tokens use `__NAME__`, `__VERSION__`, and uppercase dependency names with punctuation replaced by underscores. Versions come from the Cargo package and repository manifests. The runtime and CLI version equality is checked by a Rust test. Generated template files use MIT-0 and include their own `LICENSE`, independent of the MIT-licensed CLI.

Run `pnpm build` from the repository root first. The npm package test packs the built runtime, creates a project through the packed CLI, replaces its dependency pins with local test tarballs, installs it, and runs its full `check` script. Shipped templates contain no tarball paths.

## Upgrade a game

`npx crucible upgrade [version]` moves a game's `crucible` and `@trioforge/crucible` to one version, reinstalls, and refreshes the template files the game has not edited. See `docs/cli.md` for the rules and exit codes.

`npx crucible upgrade` runs the game's installed, older binary. It resolves the version, bumps `package.json`, and installs. Then it hands off to the newly installed binary, which compares the old template with its own, writes the files, and prints the report. The reconcile rules live in the newer binary, so a fix to them reaches games still on an older version. The install step stays in the old binary, so a fix to how a released version installs reaches only later upgrades.

The npm package test stages the current binary at the CLI version and again at the next patch version, serves both from a local registry in a worker thread (`npm/test-registry.mjs`, which passes other packages through to npmjs), creates a game at the first version, upgrades it, and runs its `check`. The next version reuses the binary, so its shim reports the next version and rewrites the old snapshot it receives to stand in for a changed template and docs. The test also drives a failed install and its retry, and a pnpm game under `CI=true`.

## Upgrade hand-off contract

Every released binary keeps this contract with every other, because an old binary hands off to whatever version a game installs, newer or older. `src/upgrade.rs` implements it.

1. The old binary runs `node <bin> --version` in the game's root, where `<bin>` is the `crucible` bin of the installed `crucible` package. The first line of stdout must start with `crucible <semver>`, followed by a space or the end of the line. That output is frozen. The old binary stops if the version is not the target.
2. The old binary runs `node <bin> __upgrade-apply <old version> <old crucible spec> <old runtime spec>` in the game's root. `<old version>` is the `crucible` package version installed before the upgrade. The two specs are the `crucible` and `@trioforge/crucible` strings from `package.json` before the bump.
3. Stdin is one JSON snapshot of the old binary's template, then EOF.

   ```json
   {"format":1,"version":"0.1.0","owned":[{"path":"AGENTS.md","section":"## Crucible","content":"<whole rendered file>"}],"docs":{"cli":"<sha256>"}}
   ```

   `owned` lists every file the old binary's template owns. `content` is the whole rendered file, and `section` is the heading of the part it owns, or `null` for the whole file. The reading binary cuts sections with its own rules. `docs` maps each topic name to the lowercase hex SHA-256 of the topic's Markdown bytes, exactly as `crucible docs <topic>` prints them.
4. The new binary inherits the old binary's stdout and stderr. It prints the report on stdout and diagnostics on stderr. Exit 0 means the report is complete, and any other exit means the refresh did not finish. With no exit within 120 seconds, the old binary kills the process tree. On any failure the old binary exits 1 and says the template was not refreshed.
5. The new binary treats the old template as unknown, which lists differing files and writes none, when the snapshot's format is higher than its own, when the snapshot's `version` differs from `<old version>`, or when either old spec is a range it can evaluate that excludes `<old version>`.

Skew rules for the snapshot:

- A binary decodes every snapshot format up to its own. Each format has its own structs and its own decode arm, so an old format never goes through a newer struct.
- A binary refuses a format higher than its own and treats the old template as unknown. It lists the differences and writes nothing.
- Raise `FORMAT` for any change to the snapshot's fields or their meaning. Add the new structs and decode arm, and keep every older arm.

## Update the spec

The checked-in source is `spec/openapi.json` from `Roblox/creator-docs` commit `46be165f526ec23305394ac0c56a95edd2d7f7d9`. `spec/overlay.toml` contains guarded corrections and the two added operations. Every command reads the generated `spec/registry.json`. The binary embeds that registry and loads no spec at runtime.

```console
cargo xtask spec update
cargo xtask spec check
```

`spec update` fetches the latest commit that changed Roblox's OpenAPI file, downloads that revision, rebuilds the registry, and prints JSON lists of added, removed, and changed operations. Review both spec files in git. An overlay expectation mismatch fails the build and requires a review of the upstream change.

After an overlay edit, run `cargo xtask spec build`. Keep the previous registry when updating so surviving operations retain their names. The weekly workflow updates, builds, and tests on Ubuntu and Windows with read-only repository permissions and no persisted checkout credentials. A separate write job creates the update PR from the verified files, with the operation diff and both verification results in its body. The commits API uses `GITHUB_TOKEN` when present. Bot PRs made with that token do not trigger CI, so the workflow's own verification job is the check. It opens the PR even if the updated-spec tests fail, then marks the workflow as failed. Overlay build failures stop the update before a PR.

## Verify changes

Run these from `cli/`.

Publish tests use local HTTP servers and never upload to Roblox. Recovery tests replay the emitted `next` argument array through a process spawn API and check place ids and credentials, including profiles with spaces. `next` is a JSON array of strings beginning with `"crucible"`. Run it through `npx` as argv without shell parsing or joining its elements. Recovery preserves explicit `--profile` and `--verbose` flags and selects each remaining place with `--place=<name>`, including names that start with `-`.

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask spec check
node --test npm/package.test.mjs
```

The behavior tests drive the real binary against local HTTP servers. `CRUCIBLE_OC_ORIGIN` overrides Roblox origins for these tests and always prints a warning. In that mode, the download client also permits HTTP loopback URLs. It still sends no key to a download URL. API requests never follow redirects.

`tests/dev.rs` runs `crucible dev` on `examples/sample-game` with the real roblox-ts. It appends a line to `src/server/main.server.ts`, waits for that line in Rojo's `/api/read` response, and restores the file. The test needs the workspace's Node dependencies and the built runtime. Run these from the repository root first.

```console
pnpm install
pnpm --filter @trioforge/crucible build:transformer
pnpm --filter @trioforge/crucible build
```

The live suite is ignored by default. Set `ROBLOX_ASSETS_API_KEY` and run its read-only tests explicitly.

```console
cargo test --test live read_only_ -- --ignored
```

`CRUCIBLE_OC_LIVE_ASSET_ID` and `CRUCIBLE_OC_LIVE_PLACE_ID` override the read fixtures. The upload test additionally needs `ROBLOX_GROUP_ID` and the user's approval before setting `CRUCIBLE_OC_ALLOW_LIVE_UPLOAD=1` and running `cargo test --test live -- --ignored`. That test creates one 1x1 PNG decal and archives it. It records the operation in `build/live-upload-receipt.json`, resolves pending work during cleanup, archives any known created asset after an assertion failure, and verifies the archive. Unknown outcomes or failed cleanup require a manual Creations check or archival of the reported asset ID.
