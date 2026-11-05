# Boot

Start Crucible on each side with `boot`, and name the modules it loads with `load` globs.

Call `boot` once from the server entry and once from the client entry. Import it from the side's subpath. `boot` requires every module that its `load` globs match, constructs the Subsystems and behaviours those modules define, and runs their hooks.

```ts
import { boot } from "@trioforge/crucible/server";

boot({ load: ["src/server/systems/**"] });
```

```ts
import { boot } from "@trioforge/crucible/client";

boot({ load: ["src/client/systems/**", "src/shared/**/*Store.ts"] });
```

## Write load globs

`load` takes an array of string literals. Crucible's transformer reads them at compile time and replaces each one with the instance path of the folder it names. The compile fails with `CRU006` when an entry is not a string literal, when a spread or computed key follows `load`, when the glob's folder is outside the Rojo mapping, or when the glob matches no module.

A glob is a folder path followed by a pattern. The folder is everything before the last `/` ahead of the first `*`, `?`, `{` or `[`. A glob without a pattern names one file. The pattern matches instance names, so `.ts` and `.tsx` at its end are dropped. A pattern supports these forms:

- `*` matches any part of one name.
- `**` matches any number of folders.
- `{Service,Component}` matches any one of its comma-separated parts.

```ts
import { boot } from "@trioforge/crucible/server";

boot({
	load: ["src/core/Admin/server/**/*{Service,Component}.ts", "src/server/round/RoundService.ts"],
});
```

### Find a module for every glob

Each glob must match at least one module, so a typo or a renamed file fails the compile instead of leaving a Subsystem unbooted. The message is `CRU006: load glob matches no module.` on the glob.

A module here is a TypeScript file in the program that the Rojo project maps to a `ModuleScript`. The transformer matches the instance each file becomes, with the same rules `boot` uses at runtime. These do not count:

- A `.server.ts` or `.client.ts` file, which becomes a script. `boot` requires only `ModuleScript`s.
- A Luau file, or an instance from another Rojo mapping. Only TypeScript compiled through the transformer can define a class for `boot` to construct.

```ts
import { boot } from "@trioforge/crucible/server";

// CRU006 when src/server/systems holds no file whose name ends in Service.
boot({ load: ["src/server/systems/*Service.ts"] });
```

## Resolve paths

Globs resolve from the package root. That is the nearest folder at or above the tsconfig's folder that holds a `package.json`, which is the folder rbxtsc takes `node_modules` from. When no folder above holds one, globs resolve from the tsconfig's folder.

This rule lets a build compile through a generated tsconfig. A tsconfig at `build/typescript/warehouse/tsconfig.json` that extends the game's own config uses the same globs as the game, such as `src/core/Admin/server/**`.

The transformer reads the Rojo project that its `rojo` plugin option names. The option also resolves from the package root. Without the option, the transformer uses the project that rbxtsc finds in the tsconfig's folder. When rbxtsc gets `--rojo`, give the transformer the same project.

```json
{
	"compilerOptions": {
		"plugins": [{ "transform": "@trioforge/crucible/transformer", "rojo": "warehouse.generated.project.json" }]
	}
}
```

The Rojo project can map the glob's folder with one `$path`, or map each file in it with its own `$path`, as a generator does before the outputs exist. In the second form, each mapped file names the instance of the folder that holds it. The compile fails with `CRU006` when those files sit under different instances.

## Check that boot finished

When a side's boot commits, Crucible sets the `CrucibleBooted` attribute to `true`. The client sets it on the local player and the server sets it on `ServerScriptService`. The side entry exports the name as `BOOTED_ATTRIBUTE`. A boot that fails leaves the attribute unset.

```ts
import { BOOTED_ATTRIBUTE } from "@trioforge/crucible/server";

const booted = game.GetService("ServerScriptService").GetAttribute(BOOTED_ATTRIBUTE) === true;
```

Each side also writes one Debug record from the `Crucible.Boot` logger, `Crucible booted on the server` or `Crucible booted on the client`. The default output prints Debug records in Studio, so the line appears in Studio's output without a sink.
