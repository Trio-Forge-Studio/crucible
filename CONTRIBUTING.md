# Contributing to Crucible

This guide covers reporting a problem, setting up the workspace, and opening a pull request. Everyone in the project follows the [code of conduct](CODE_OF_CONDUCT.md).

## Report a problem

Open a [GitHub issue](https://github.com/Trio-Forge-Studio/crucible/issues). Include the output of `npx crucible --version`, the code or command you ran, what you expected, and what happened. For a compile error, include its `CRU` code. To propose a large change, open an issue before you write the code.

## Set up the workspace

Install Node.js 24, pnpm 12.15.0, [Rokit](https://github.com/rojo-rbx/rokit), and [rustup](https://rustup.rs). `rokit.toml` pins the Roblox tools, and `rust-toolchain.toml` pins Rust. Then run these commands from the repository root.

```sh
pnpm install --pm-on-fail=download
rokit install
pnpm check
```

`pnpm install` also installs the git hooks. `pnpm check` must exit 0 before you open a pull request. It checks formatting and lint, builds the runtime and the sample game, and runs the specs. `pnpm fmt` fixes formatting.

The repository has these parts:

- `packages/crucible` is the runtime package, `@trioforge/crucible`. Its root entry, `src/index.ts`, loads on both sides, so it must never import server or client code.
- `crates/cli` holds the `crucible` CLI and `crates/compiler` holds Crucible's compiler, in Rust. `crates/cli/README.md` covers building and packaging the CLI.
- `docs` holds the guides that `crucible docs` prints.
- `examples/sample-game` is the game that the specs build and load.
- `test` holds the Lune specs.

## Make a change

1. Create a branch named `<type>/<short-description>`, such as `docs/fix-boot-example`.
2. Keep the pull request to one topic.
3. Add or update a spec for each behavior you change.
4. If you change a feature that a guide describes, update the guide in `docs`. The specs type-check each TypeScript block in the guides.
5. Run `pnpm check`. If you changed files under `crates` or `xtask`, also run `pnpm build` from the root, then these commands from `.`:

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo xtask spec check
   cargo test --workspace
   node --test crates/cli/npm/package.test.mjs
   ```

6. Write commit messages and the pull request title as [Conventional Commits](https://www.conventionalcommits.org) with a scope, such as `docs(boot): fix the load glob example`.
7. Open the pull request into `main` and fill in the template.

The specs fake only the engine calls that the sample game makes, so a passing spec does not prove the game works in Studio. If your change affects gameplay, test it in Studio and describe what you saw in the pull request.

A maintainer reviews each pull request and squash-merges it. Your contribution is under the [MIT license](LICENSE), like the rest of Crucible.
