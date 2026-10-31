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

`npm/stage.mjs` writes the packages from built binaries. Every package takes its version from `[workspace.package]` in `Cargo.toml`. The packages stay `private` until a release task publishes them.

```console
node npm/stage.mjs build/npm win32-x64=target/release/crucible.exe
```

`npm/package.test.mjs` stages the debug binary from `cargo test`, packs the packages with `npm pack`, installs them into a temporary project, and runs `npx crucible`. Set `CRUCIBLE_BINARY` to test another binary.

## Create a game

`npx crucible create my-game [path]` writes the embedded game template and installs dependencies. `--no-install` skips install. See `docs/cli.md` for arguments and exit codes.

In the generated game, run `npx crucible dev` to compile on save and sync the game to Studio.

`crates/crucible/template` holds the real template files. `src/create.rs` embeds an explicit file table. Tokens use `__NAME__`, `__VERSION__`, and uppercase dependency names with punctuation replaced by underscores. Versions come from the Cargo package and repository manifests. The runtime and CLI version equality is checked by a Rust test.

Run `pnpm build` from the repository root first. The npm package test packs the built runtime, creates a project through the packed CLI, replaces its dependency pins with local test tarballs, installs it, and runs its full `check` script. Shipped templates contain no tarball paths.

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
