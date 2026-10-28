# Agent instructions

Crucible is Trio Forge's roblox-ts game framework. It replaces Flamework and will own the game runtime. This repository currently proves the package layout with placeholders. Do not add framework features as part of scaffold maintenance.

## Layout

- `packages/crucible` is the single runtime package, `@trioforge/crucible`.
- `packages/crucible/src/index.ts` is the shared-safe root. It must never import or re-export server or client code, including through transitive imports.
- `packages/crucible/src/server/index.ts` and `src/client/index.ts` are the side entries. Physical `server` and `client` folders in the package resolve these entries for roblox-ts 3.0.0. Keep them alongside the package exports map. An exports-only remap does not resolve with this compiler.
- `examples/sample-game` imports the root and each side entry through workspace dependencies. Its Rojo project maps the runtime under `ReplicatedStorage.rbxts_include.node_modules.@trioforge`.
- `test` holds Lune specs. `.lune/test.luau` uses the `@trioforge/tooling` runner. The specs load the built place through roblox-ts's runtime and assert subpath resolution and root isolation.

## Checks

Run `pnpm install --pm-on-fail=download` and `rokit install`. Keep the lockfile after dependency changes. Rokit pins the Roblox tools. Vite+ configures TypeScript lint and formatting in `vite.config.ts`.

Run `pnpm check` before calling a change complete. It must exit 0. It checks formatting and lint, compiles the runtime and sample, builds `build/sample-game.rbxl`, and runs the Lune specs. `pnpm fmt` fixes TypeScript, configuration, and Luau formatting. Fix failures at their cause.

The Lune loader supplies only the engine behavior needed to execute the sample imports. A passing Lune spec does not prove gameplay behavior in Studio.

## Conventions

- Use Conventional Commits with a scope, such as `chore(repo): scaffold the workspace`.
- Name branches `<type>/<short-description>` and open PRs into `main`.
- Push with an explicit refspec, such as `git push origin HEAD:refs/heads/chore/scaffold`.
- Use squash merge and delete the branch after merge. Do not merge without task authorization.
- Comment only a non-obvious reason that the code cannot express.
- Keep the package private and unlicensed. Publishing, release workflows, and repository settings require a separate task.
