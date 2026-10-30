# Game development

Run `npm run build` to compile TypeScript and build `build/start.rbxl` with embedded Rojo.
Run `npm run check` before completing a change. It runs Vite+ lint and formatting, then builds the game.
Run `npm run fmt` to fix formatting. Use the equivalent script commands with your package manager.

## Crucible

Read `npx crucible docs` and `npx crucible docs cli` first.
Before using a module, read `npx crucible docs <topic>`, such as `npx crucible docs logging`. Documented exports are the source of truth.

`@trioforge/crucible` is shared-safe. Shared code must never import or re-export `/server` or `/client`, including transitively.
Only server code imports `@trioforge/crucible/server`. Only client code imports `@trioforge/crucible/client`.
Keep entries in `src/server` and `src/client`. Call the matching side's `boot` with system file globs in `load`.
Use `Logging.getLogger` with an explicit dotted name once per module. Logging works before boot and stays local to each Luau VM.

`crucible.toml` holds place and environment IDs, never credentials.
