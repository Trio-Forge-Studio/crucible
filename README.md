# Crucible

Crucible is Trio Forge's roblox-ts game framework. This private workspace contains a runtime package, a sample game that proves package resolution, and the `crucible` command line in `cli`. Framework features and publishing come in later units.

## Check the workspace

Install Node.js 24, pnpm 12.15.0, and Rokit. Run these commands from the repository root.

```sh
pnpm install --pm-on-fail=download
rokit install
pnpm check
```

`pnpm check` checks formatting and lint, compiles both workspace packages, builds `build/sample-game.rbxl`, and runs the Lune specs against that place. CI runs the same command with the shared Trio Forge setup action. It omits the reusable workflow's changeset job because this private scaffold has no release flow.

Run `pnpm build` to build the sample place without the checks. Run `pnpm fmt` to fix formatting.

## Command line

`cli` holds the `crucible` binary, written in Rust. `npx crucible docs cli` lists every command. See `docs/cli.md` for the commands and `cli/README.md` for building and testing the CLI.

## Package entries

The sample server imports `@trioforge/crucible` and `@trioforge/crucible/server`. The sample client imports the root and `@trioforge/crucible/client`. Both scripts store the imported placeholders as attributes so the specs can assert their values.

The root stays shared-safe. Requiring it must never load server or client code. The package keeps physical subpath folders because roblox-ts 3.0.0 does not resolve an exports-only remap. The Rojo project maps the package under `ReplicatedStorage.rbxts_include.node_modules.@trioforge`.

The sample replicates the package's files to prove resolution. Subpath names do not enforce client access restrictions. The Lune loader runs the compiled imports with a small engine adapter, not a Roblox game simulation.
