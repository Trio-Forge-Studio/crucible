# Game development

Run `npm run build` to compile TypeScript and build `build/start.rbxl` with embedded Rojo. Ask a person to open it in Studio.
Run `npm run check` before completing a change. It runs Vite+ lint and formatting, then builds the game.
Run `npm run fmt` to fix formatting. Use the equivalent script commands with your package manager.

## Crucible

Read `npx crucible docs` and `npx crucible docs cli` first. Read `npx crucible docs lifecycle` before you write a Subsystem or a behaviour. It covers hooks, their order, injection, faults and the yield rule.
When a build fails with a `CRUnnn` code, read `npx crucible docs diagnostics`. Plain `tsc` does not report these codes, so run `npm run build` or `npm run check` to check Crucible's rules.
Run `npx crucible upgrade` to move to the latest Crucible. It refreshes template files you have not edited, lists the ones you did, and names the docs to read again.
Before using a module, read `npx crucible docs <topic>`, such as `npx crucible docs logging`. Documented exports are the source of truth.

`@trioforge/crucible` is shared-safe. Shared code must never import or re-export `/server` or `/client`, including transitively.
Only server code imports `@trioforge/crucible/server`. Only client code imports `@trioforge/crucible/client`.
Keep entries in `src/server` and `src/client`. Call the matching side's `boot` with system file globs in `load`.
Put modules used by both sides in `src/shared`, which Rojo maps to `ReplicatedStorage.TS`. `src/shared/game.ts` exports the game name for both entry scripts.
For example, the server entry can load systems with the literal source glob below. The client entry uses `@trioforge/crucible/client` and `src/client/systems/**`.

```typescript
import { boot } from "@trioforge/crucible/server";

boot({ load: ["src/server/systems/**"] });
```

Use `Logging.getLogger` with an explicit dotted name once per module. Logging works before boot and stays local to each Luau VM.

`crucible.toml` holds place and environment IDs, never credentials.
