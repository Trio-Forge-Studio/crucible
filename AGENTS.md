# Agent instructions

Crucible is Trio Forge's TypeScript game framework. It replaces Flamework and will own the game runtime. This repository currently proves the package layout with placeholders. Do not add framework features as part of scaffold maintenance.

## Layout

- `packages/crucible` is the single runtime package, `@trioforge/crucible`.
- `packages/crucible/src/index.ts` is the shared-safe root. It must never import or re-export server or client code, including through transitive imports.
- `packages/crucible/src/server/index.ts` and `src/client/index.ts` are the side entries. The package exports map resolves side entries under TypeScript 7 bundler resolution.
- `packages/crucible/src/vide/index.ts` is `@trioforge/crucible/vide`, the bridge from Crucible readables to Vide sources. It is the only module that imports `@rbxts/vide`, an optional peer dependency. No other entry may import it.
- `packages/crucible/src/ui/index.ts` is `@trioforge/crucible/ui`, Crucible UI's runtime: `mount`, `Show`, `For`, and the `bind` and `append` calls that compiled JSX makes. `src/ui/jsx-runtime.ts` holds the JSX types, which TypeScript reads through `jsxImportSource`. The exports map also resolves its JSX types. Only the Crucible compiler builds JSX for it, so its fixtures live in `cli/crates/compiler/tests/jsx/ui`, not in the sample game.
- `packages/crucible/src/util/random/index.ts` is `@trioforge/crucible/util/random` and `src/maths/spline/index.ts` with `curve.ts` is `@trioforge/crucible/maths/spline`. They are the retired `@trioforge/random` and `@trioforge/spline` packages, moved in unchanged, because `Rng` draws the same numbers from a seed on every client and server and games save its state. Do not change what either draws or returns. Neither imports anything, and no other entry may import them. There is no `util` or `maths` entry. The compiler allows importing both under CRU004. `test/lib/random-golden.luau` and `test/lib/spline-golden.luau` hold answers recorded from the published packages, so a change that moves an answer fails the specs.
- `packages/crucible/src/testing/index.ts` is `@trioforge/crucible/testing`, which builds one Subsystem or behaviour in a Lune spec without `boot`. Game code cannot import it, because CRU004 rejects it. Only specs that load the built game import it, and no other entry may.
- `examples/sample-game` imports the root and each side entry through workspace dependencies. Its Rojo project maps the runtime's `out` folder under `ReplicatedStorage.rbxts_include.node_modules.@trioforge.crucible`. Mapping the whole package folder would add the package's own `node_modules`, and `TS.getModule` would find its copy of Vide before the game's.
- `docs` holds the user guides, one plain Markdown topic per file. The `crucible` binary embeds them. `crucible docs <topic>` prints one as Markdown when stdin or stdout is not a terminal, and opens it in an interactive reader when both are. A `mermaid` fence must render with `mermaid-text`, or the CLI build fails. Each topic starts with `# Title`, a blank line, and a one-line summary that ends with a period. Document only what the code exports today. `test/docs.test.mjs` type-checks every `ts`, `typescript`, and `tsx` block against the runtime package.
- `test` holds Lune specs. `.lune/test.luau` uses the `@trioforge/tooling` runner. The specs load the built place through roblox-ts's runtime and assert subpath resolution and root isolation.
- `packages/compiler-types` is `@trioforge/compiler-types`, the compiler types with the three iterator parameters TypeScript 7 requires. All tsconfigs use it. It releases with the CLI. `test/compiler-types.test.mjs` checks the runtime and sample with tsrs.
- `cli/crates/compiler` is `crucible-compiler`, the TypeScript 7 compiler in `design/compiler.md`. It checks a tsconfig with tsrs, fills a fact table, and lowers each file to Luau from the syntax tree and the facts alone. The fact pass also runs Crucible's transform: the CRU001 to CRU037 rules, class registration, `attribute`, `inject`, `boot`, `receive` and the network, replicated, data and config declaration macros. JSX lowers to `jsxFactory` calls under `jsx: "react"`, and to instance code under `jsx: "react-jsx"` with `jsxImportSource: "@trioforge/crucible/ui"`. `compile` and `analyze` take a `Target`: `Package`, or `Game` with the Rojo project and the include folder, which reaches packages and the runtime library through the place. The compiler crate vendors roblox-ts 3.0.0's `RuntimeLib.lua` and `Promise.lua` in `roblox-ts/` with roblox-ts's MIT license and, for `Promise.lua`, roblox-lua-promise's, and `write_include` writes them. tsrs types stay in its `program`, `extract` and `tree` modules. `cargo run -p crucible-compiler --example compile -- <tsconfig> <out> [<project> <include>]` writes the Luau. `crucible build` compiles each place of `crucible.toml` in a hidden `crucible __compile` child, because tsrs never frees a program. Its tests read `packages/crucible/out` and `build/sample-game.rbxl`, so run `pnpm build` first. The tests that run the emitted Luau need `lune` on PATH and are ignored by default, so run them with `cargo test -p crucible-compiler -- --ignored`. CI runs them in the Linux `cli` job. `tests/transform.rs` and `tests/network.rs` own the transform, network, replicated, data and config tests. Their fixtures live in `tests/transform-fixtures`. `pnpm run test:compiler` builds the runtime and the sample game with the compiler and runs the Lune specs against them in a temporary directory, after `pnpm build`. `pnpm check` builds both artifacts with the compiler. CI also reruns the isolated compiler specs.
- `cli` is the `crucible` command line, a separate Cargo workspace. `crucible oc` calls Roblox Open Cloud and keeps the JSON-document and exit-code contract in `docs/oc.md`. `cli/spec/registry.json` is generated by `cargo xtask spec build`. Do not edit it by hand.

## Checks

Run `pnpm install --pm-on-fail=download` and `rokit install`. Keep the lockfile after dependency changes. Rokit pins the Roblox tools, and the root `rust-toolchain.toml` pins Rust to the version the pinned tsrs SHA requires. CI installs Rust from that file. Vite+ configures TypeScript lint and formatting in `vite.config.ts`.

Run `pnpm check` before calling a change complete. It must exit 0. It checks formatting and lint, compiles the runtime with the compiler example in package mode, emits its declarations with tsrs, builds the sample into `build/sample-game.rbxl` with `crucible build` through `cargo run`, and runs the Lune specs. `pnpm fmt` fixes TypeScript, configuration, and Luau formatting. Fix failures at their cause.

For changes under `cli`, run `pnpm build` from the repository root before the npm package test. Also run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask spec check`, `cargo test --workspace`, and `node --test npm/package.test.mjs` from `cli`. CI runs them on Ubuntu and Windows.

The Lune loader supplies only the engine behavior needed to execute the sample imports. A passing Lune spec does not prove gameplay behavior in Studio.

## Cleanup

Every Cargo build writes `cli/target` in its own worktree, and each one grows to several gigabytes. Do not set `CARGO_TARGET_DIR` or clone the repository into another folder, such as `C:\build` or a temporary directory, because nothing cleans those up. Before you hand back a task that ran Cargo, run `cargo clean` from `cli`, whether the task passed, failed, or stopped. Leave the worktree itself in place, because the thread that owns it may resume. Never clean another agent's worktree.

## Conventions

- Use Conventional Commits with a scope, such as `chore(repo): scaffold the workspace`.
- Name branches `<type>/<short-description>` and open PRs into `main`.
- A PR that resolves a Linear issue puts `Closes <ID>` in its body, such as `Closes CRU-12`. Branch names carry no issue ID, so the body links the PR to the issue.
- Push with an explicit refspec, such as `git push origin HEAD:refs/heads/chore/scaffold`.
- The pre-commit hook runs staged checks, and the pre-push hook refuses pushes to `main`; `pnpm install` installs both.
- Use squash merge and delete the branch after merge. Do not merge without task authorization.
- Comment only a non-obvious reason that the code cannot express.
- The runtime and CLI use MIT. Publish npm packages only through the tag release workflow. Publishing a release, creating secrets, pushing release tags, and changing repository settings require separate authorization.
