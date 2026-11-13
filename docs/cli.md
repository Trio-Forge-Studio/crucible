# Crucible CLI

Run `crucible` commands and read their output and exit codes.

`crucible` is the command line for Crucible games. It ships as the `@trioforge/crucible-cli` npm package. Run it with `npx crucible <command>` from a project that has `@trioforge/crucible-cli` in its `devDependencies`. `npx crucible docs cli` lists every command.

`crucible --help` lists the commands, and `crucible <command> --help` describes one. `crucible --version` prints the CLI version and the embedded Rojo version.

## Output and exit codes

Commands print text. The result goes to stdout, and progress and errors go to stderr. No command prompts for input. `crucible oc`, `crucible deploy`, and `crucible whoami` print one JSON document per command instead of text and have their own exit codes, described in `crucible docs oc`. `crucible provision` and `crucible adopt` use those exit codes too, and print one JSON document with `--json`.

| Exit | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The command failed. |
| 2 | The arguments are invalid. The usage error is on stderr. |

## crucible build

Run `npx crucible build` from the game's root. It compiles every place in `crucible.toml` with Crucible's TypeScript 7 compiler, then builds each one to `build/<name>.rbxl` with Rojo. The binary embeds both, so the build runs no Node, roblox-ts, or TypeScript, and projects need no separate Rojo install. `crucible --version` names the Rojo version.

- A place compiles its `tsconfig` against its Rojo `project`. Type errors, Crucible rule errors (`CRUnnn`), and code the compiler cannot lower reach stderr as `file(line,column): error ...`, with paths relative to the game's root.
- When any selected place fails to compile, the build writes nothing and builds no place file. Places compile one after another, and the first failure stops the build.
- The build writes the compiled Luau into the tsconfig's `outDir` and skips each file whose bytes already match. It deletes the files it compiled earlier that no longer have a source, and keeps every other file there.
- It writes roblox-ts 3.0.0's `RuntimeLib.lua` and `Promise.lua` into the place's `include` folder, which the project must map. A file whose bytes already match is left alone, and other files there stay.
- Places that name the same `tsconfig`, `project`, and `include` compile once. Before anything compiles, the build reads the `outDir` of every place in `crucible.toml`, including places `--place` leaves out. When a place's tsconfig, or a config it extends, cannot be read or parsed, or it sets no `outDir`, the build exits 1 and names the place and the file. Two places that share an `outDir` but differ in `tsconfig`, `project`, or `include` exit 1, because their code would overwrite each other's. Two places with one `outDir` inside the other also exit 1. Folder names that differ only in case count as the same folder on every platform.
- Each place compiles in its own process, so a build's peak memory is that of its largest place.
- `--place <name>` builds only that place. Repeat it to build several.
- `--output <file>` writes the place file elsewhere. It needs exactly one place.
- `--env <name>` binds `crucible:experience` to that environment's IDs in `crucible.lock`, as `npx crucible docs experience` describes under "Use the handles in game code". `crucible deploy <env>` builds with its environment.
- Exit 0 means every place built. Exit 1 means `crucible.toml` could not be read, a place failed to compile or write, no place grants a declared product and none answers `ProcessReceipt` itself, or Rojo failed. Exit 2 means a usage error or an invalid `crucible.toml`, reported before anything compiles.

`crucible.toml` lives at the game's root. It holds ids, never keys. Without the file, or without `[places]`, the game has one place, `start`, built from `tsconfig.json`, `default.project.json`, and the `include` folder at the root.

A place names its Rojo `project`. `tsconfig` defaults to `tsconfig.json` and `include` defaults to `include`. All three paths are relative to the game's root. A place with its own tsconfig, project, or include needs a tsconfig whose `outDir` no other place writes.

```toml
[places.start]
project = "default.project.json"

[places.arena]
tsconfig = "tsconfig.arena.json"
project = "arena.project.json"
include = "places/arena/include"

[env.staging]
universe = 1234
places = { start = 5678, arena = 5679 }

[env.production]
universe = 9999
version_type = "Published"   # default "Saved"
places = { start = 4321, arena = 4322 }
```

An env that maps places must map every declared place, and only declared places. A game whose only place is `start` can write `place = 4321` in an env instead of `places`. Such a game can also omit `[places]`. An env may set neither `place` nor `places` when crucible never publishes it, such as for a game that publishes its places with its own scripts: `crucible plan`, `provision`, `adopt` and `build --env` need only its `universe`, and `crucible deploy` refuses it with exit 2 before any request. Place and env names use letters, digits, `-` and `_`. Unknown keys are errors.

### One reactive library copy per place

Before compiling or writing, `crucible build` checks each selected place's Rojo mappings for multiple copies of `@rbxts/charm`, `@rbxts/vide`, or `@rbxts/charm-sync`. Charm and Vide keep reactive state at module scope; Charm Sync keeps client subscriptions and server tracking there. Separate copies split that state. `vide-charm` has no independent shared mutable state, so its copies are allowed, but its declared ranges are checked.

The check reads only what the place contains. It follows the place's Rojo tree, including nested project files and `globIgnorePaths`, through the mapped folders, and finds each folder with a `package.json` in them, then that package's nested `node_modules`. A mapping of a package's `src` or `out` folder also counts as that package. Each instance of a library in the tree is a copy, so one folder mapped at two places is two copies, while links that reach one folder at one place are one. A copy on disk that the tree does not reach does not count, and neither does the `package.json` of a package outside the place. The `dependencies` and `peerDependencies` of each package in the place, and of the game's own `package.json`, supply version ranges for the libraries. Each range is checked against the copy that its package loads: the nearest `node_modules` copy up the instance tree, else a sibling, as the Luau bridges require it. The game's own range is checked against the copy its `node_modules` resolves to. A `package.json` in the place that is not valid JSON fails the build. Absent libraries are not required by this check, so games without Vide or `vide-charm` still build. An entry that names another source, such as `file:`, `link:`, a Git or URL spec, a tag, `catalog:` or a bare `workspace:`, declares no range; the installed copy's own version counts. An `npm:` alias of the same package and `workspace:^1.2.3` keep the range they hold.

A failure exits 1 before anything is written. It names each copy and declared range with its version and its path from the game, then one fix that uses the package manager the game's lockfile names, as `crucible upgrade` finds it. For a copy nested under another package, make the versions agree so the manager installs one copy: give the library a version that package accepts, or upgrade that package, then reinstall. When the nested copy has the same version, as under a linked workspace package, reinstalling changes nothing: map only that package's built folder, such as `out`, or list its `node_modules` under `globIgnorePaths` in the Rojo project. For a second folder the project maps, keep one: remove the other's mapping, or the dependency that installs it, then reinstall. For a version outside a range, change the game's version to one inside it, or upgrade the package that declares it, then reinstall. An override, such as npm `overrides`, pnpm's `overrides` or Yarn's `resolutions`, forces one version but leaves the ranges packages declare, so a range that fails still fails. Map Crucible's `out` folder rather than its entire package when its development dependencies would otherwise enter the place.

## crucible create

Run `npx @trioforge/crucible-cli create my-game` to write a game into `./my-game` and install its dependencies. An optional path selects another directory, such as `npx @trioforge/crucible-cli create my-game games/my-game`. The target must be absent or empty. Invalid names and occupied targets exit 2 without writing files. Names and path components cannot end in a dot or space or use Windows device names, including names with extensions such as `con.txt`. A target below an existing file also exits 2.

The game pins the CLI, the runtime, and `@trioforge/compiler-types` at the CLI version. It lists `@rbxts/charm`, `@rbxts/charm-sync`, `@rbxts/vide`, `@rbxts/vide-charm`, `@rbxts/ripple` and `@rbxts/vide-ripple` in `dependencies` at the versions the CLI was released with, and includes the Roblox types, Vite+, side entry scripts, a TypeScript 7 tsconfig that compiles JSX for Vide, and a Rojo project. Rojo maps the compiled `src/shared` folder to `ReplicatedStorage.TS`, so both sides can import modules in it. The mapping is optional, so a game without the folder still builds. Template files use MIT-0. No roblox-ts, TypeScript, Rojo, or Rokit install is needed.

The command detects npm, pnpm, Yarn, or Bun from `npm_config_user_agent`, with npm as the default. Yarn projects include `.yarnrc.yml` with `nodeLinker: node-modules`, because the compiler and the Rojo project read packages from `node_modules`. `--no-install` writes files without running install. Install failures exit 1 and leave the project available for a manual retry. Progress and install output go to stderr. Success prints the project path to stdout.

In the new directory, run `npx crucible build`, then open `build/start.rbxl` in Studio.

Run `npm run check` to lint, check formatting, compile, and build `build/start.rbxl`. `npm run build` builds the game, and `npm run fmt` fixes formatting. Use the equivalent script commands with your package manager. Read the generated `AGENTS.md` and `npx crucible docs cli` before developing the game.

## crucible deploy

`npx crucible deploy production` checks that `crucible.lock` records every product, pass and badge `crucible.toml` declares for `[env.production]`, and warns when it does not record the experience settings yet, then builds the game and uploads every place to the ids in `[env.production]`. It never creates or changes a product, a pass, a badge or an experience setting: run `npx crucible provision production` and commit `crucible.lock` first. Run `npx crucible docs deploy` for the checks, their order and the release flow. `crucible publish`, which 0.6.0 replaced, prints a usage error that names `crucible deploy` and exits 2.

`--place <name>` selects a place and can repeat. Duplicate selections upload once. Places run in byte order by name. All selected places build before the first upload. The command stops at the first failed upload and does not roll back earlier uploads.

`--dry-run` runs the same checks and builds the selected places, and sends no upload request. `--no-scope-check` skips the check of the key's scopes.

The key comes from `CRUCIBLE_OC_API_KEY` in the environment, or from `.env` at the game's root, as [Configure a key](oc.md#configure-a-key) describes. A missing or invalid key fails without trying another. `--verbose` writes redacted request diagnostics to stderr. `version_type` defaults to `Saved`; set `Published` in the environment to publish the new versions.

Deploy prints exactly one JSON document on stdout. Build progress and compiler diagnostics go to stderr. It uses the [Open Cloud exit codes](oc.md#exit-codes), including usage 2, auth 3, rejected 4, outcome_unknown 5, remote_failure 6, unavailable 7, and pending 8. Build failures are unavailable, exit 7. A missing, unreadable or invalid `crucible.toml`, `crucible.lock` or journal is usage 2, and so are unknown environments or places, with valid names.

Success, exit 0, includes each confirmed upload's version number. Ids are strings and version numbers are integers.

```json
{"environment":"production","dryRun":false,"published":[{"name":"arena","universeId":"9999","placeId":"4322","versionNumber":7,"versionType":"published"},{"name":"start","universeId":"9999","placeId":"4321","versionNumber":8,"versionType":"published"}],"failed":null,"notAttempted":[]}
```

An upload failure keeps the existing Open Cloud error fields and puts progress under `error.deploy`. `failed` names the place whose upload failed. Its outcome can be unknown. `published` contains only confirmed receipts, and `notAttempted` lists the selected places after the failed place. The exit code classifies the failed upload independently of earlier successes. `next` is a JSON array of string arguments beginning with `"crucible"`. It selects the failed place and every unattempted place with `--place=<name>` and preserves an explicit `--verbose` flag. It does not copy environment variables, so the replay reads the key again from the environment or `.env`. Pass `next` as argv to a process spawn API without shell parsing or joining its elements. With an installed npm package, run it through `npx` with `next` as the argument array.

After a partial failure, never rerun the environment without `--place`, because that uploads confirmed places again. For `outcome_unknown`, check the failed place's versions before re-uploading it. Uploads follow the [place publishing retry policy](oc.md#publish-a-place).

```json
{"error":{"kind":"rejected","message":"HTTP 400","status":400,"detail":{},"deploy":{"environment":"production","dryRun":false,"published":[{"name":"arena","universeId":"9999","placeId":"4322","versionNumber":7,"versionType":"published"}],"failed":"start","notAttempted":[],"next":["crucible","deploy","production","--place=start"]}}}
```

Credential, scope and build failures have the same progress object, with `failed` null and all selected places in `notAttempted`. A refused check of the files returns the usual `{"error":{"kind":"usage","message":"..."}}` before reading the key, with `steps` or `problems` when the plan caused it.

A successful dry run exits 0 with no published receipts. `auth` is the `crucible whoami` result, or `null` when the introspection failed or `--no-scope-check` skipped it. All selected places remain unattempted.

```json
{"environment":"production","dryRun":true,"published":[],"failed":null,"notAttempted":["arena","start"],"auth":{"keySource":"CRUCIBLE_OC_API_KEY","name":"deploy-key","enabled":true,"expired":false,"scopes":["universe-places:write"]}}
```

## crucible docs

Prints a guide as Markdown, or opens it in a reader at a terminal. Read the guide for a module before you write code that uses it.

```console
npx crucible docs
npx crucible docs logging
npx crucible docs logging --plain
```

When stdin or stdout is not a terminal, `crucible docs` prints Markdown. Without a topic, it prints one line per topic with its name and summary. With a topic, it prints that guide to stdout. `--plain` and `TERM=dumb` print the same bytes at a terminal. An unknown topic exits 2 and lists the valid topics on stderr. The guides are compiled into the binary, so `crucible docs` needs no network.

When stdin and stdout are both a terminal, `crucible docs` opens a reader instead. Without a topic, it opens on the list of topics. With a topic, it opens that guide. The reader highlights code, draws tables and Mermaid diagrams, and shows the topic, section, and main keys on the bottom line. A terminal at least 90 columns wide also shows the topics in a sidebar. Click a topic or a link to open it. A link to another guide opens in the reader, and a web link opens in the browser. Quitting restores the terminal.

| Key | Action |
| --- | --- |
| `↑` `↓`, `j` `k`, mouse wheel | Scroll. |
| `PageUp` `PageDown` `Space` | Scroll a page. |
| `g` `G`, `Home` `End` | Go to the top or the bottom. |
| `Tab`, `Shift+Tab` | Focus the next or previous link. |
| `Enter` | Follow the focused link. |
| `Backspace`, `Alt+←` | Go back. |
| `Alt+→` | Go forward. |
| `Ctrl+K`, `/` | Search every guide. Type to filter, `↑` `↓` to choose, `Enter` to open the section, and `Esc` to close. |
| `?` | Show the keys. |
| `q`, `Esc`, `Ctrl+C` | Quit. |

## crucible oc

Calls Roblox Open Cloud. It uploads and downloads assets, publishes places, runs Luau, and calls any API-key operation in the Roblox spec. Each command prints one JSON document to stdout.

```console
npx crucible oc find data store entry
npx crucible oc place publish build/game.rbxl --universe 13058 --place 1818
```

Run `npx crucible docs oc` for keys, commands, and exit codes. `npx crucible oc --help` prints a short form.

## crucible plan

`npx crucible plan [<env>]` prints what `crucible provision` would do to the products, passes and badges `crucible.toml` declares, in one environment or in all of them. It reads `crucible.toml`, `crucible.lock` and `.crucible/journal/<env>.json`, makes no network call and writes nothing. `--json` prints one document instead of text. It exits 0 when the plan prints, 2 when a file is invalid or the plan breaks a provisioning rule, and 1 when a file cannot be read. Run `npx crucible docs experience` for the declarations, the actions and the rules.

## crucible provision

`npx crucible provision <env>` lists the products, passes and badges in the environment's universe, prints the plan, and applies it: it adopts an existing resource by its name, creates the rest, updates what differs, and records each ID in `crucible.lock`. `--dry-run` lists and plans and writes nothing, and `--no-scope-check` skips the check of the key's scopes. `npx crucible adopt <env> <kind> <key> <id>` records one resource for a key when a run stopped on two candidates or on a badge whose create may have landed. Both read the Open Cloud key like `crucible oc`. Run `npx crucible docs provision` for the steps, the journal, the stops and the exit codes.

## crucible upgrade

Run `npx crucible upgrade` from the game's root to move `@trioforge/crucible-cli` and `@trioforge/crucible` to the latest version in the registry. Pass a version, such as `npx crucible upgrade 0.2.0`, to pick one.

```console
npx crucible upgrade
npx crucible upgrade 0.2.0
```

- The latest version comes from `npm view @trioforge/crucible-cli version` in the game's root, so the registry and credentials in `.npmrc` apply. A game where either package is already newer, such as a prerelease, exits 0 and changes nothing.
- Both packages move to the same exact version in `package.json`, and so does `@trioforge/compiler-types` when the game pins it. Only their version strings change, so the file keeps its formatting and line endings. A `workspace:`, `file:`, `link:`, `npm:` alias, Git, URL, path, or tag spec exits 2, because only a registry version or range can be rewritten. Any npm range is accepted, such as `^0.1.0 || ^0.2.0`, `>=0.1.0 <0.2.0`, or `0.1.0 - 0.2.0`.
- The game's package manager installs them. The nearest lockfile in the game's root or a parent directory picks the manager, so a workspace member installs with its workspace's manager: `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `bun.lockb`, or `package-lock.json`. Without one, `npm_config_user_agent` picks it, with npm as the default. pnpm runs with `--no-frozen-lockfile` and Yarn Berry with `--no-immutable`, because under CI both refuse a changed `package.json` by default.
- On Windows the `crucible` shim runs the upgrade from a copy of the binary in the temporary folder and deletes the copy afterwards. The install replaces `node_modules/@trioforge/crucible-cli-win32-x64`, and Windows cannot delete a binary while it runs. The shim of 0.5.0 and older runs the binary in place, so [an upgrade from those versions](#upgrading-from-050-or-older-on-windows) can need a step.
- A version older than either installed package exits 2 unless you pass `--allow-downgrade`. When both pins and both installed packages are already at the version, it exits 0 after reconciling runtime peers. If `package.json` still lists `roblox-ts` or `@rbxts/compiler-types`, it finishes [the move off roblox-ts](#upgrading-from-roblox-ts) without reinstalling crucible. Otherwise it installs again, so a retry after a failed install finishes the job.
- After install, both installed packages and the installed binary must report the new version, or the command exits 1 with the versions it found.
- The template owns `tsconfig.json`, `default.project.json`, `vite.config.ts`, `.gitignore`, `.gitattributes`, `LICENSE`, and the `## Crucible` section of `AGENTS.md`. A file or section that still matches the old version's template takes the new version's content and keeps the file's line endings. One that differs is listed and left alone. A deleted one is listed and not recreated. A part the new template no longer includes is listed for you to delete. The comparison ignores CRLF line endings. `package.json` belongs to the game, and the upgrade changes only the entries this section and [Upgrading from roblox-ts](#upgrading-from-roblox-ts) name. `crucible.toml` and `src` are never changed.
- Before the version install, upgrade reads the target runtime's `peerDependencies` with `npm view` in the game's root and reconciles the game's Charm and Charm Sync pins, so an incompatible old peer cannot block npm from installing the target runtime. The game's installed version runs this step, so an upgrade from 0.4.0 or older skips it. After the hand-off, the new binary reconciles its own peer table and bridge pin and installs any remaining changes using the detected package manager. It also reconciles Vide and `vide-charm` when the game imports `@trioforge/crucible/ui` or already lists either UI package; games without UI gain neither. A compatible exact version or the required range stays unchanged. A peer the game lacks goes where the package manager sorts it: before the first entry of its group whose name sorts after it, with that entry's indentation and line ending. `crucible create` uses the same version table. If peer installation fails, the manifest is restored; run the named command and upgrade again.
- The new binary removes `@trioforge/anvil` from `package.json` when no code file and no `*.project.json` file in the game names it, and installs with the peers. It reads `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs` and `.cjs` files, follows linked folders, and skips `node_modules`, `out`, `build`, `target` and folders whose name starts with a dot. Any mention counts, a comment included, because removing a package the game still uses breaks its build. Its exports moved to `@trioforge/crucible/ui`, and a game that kept it would load a second copy of the primitives on another reactive core. When a file still names it, the package stays, and the report names that file and says to move the imports and delete the Rojo mapping, as `npx crucible docs ui` explains under Move from `@trioforge/anvil`. The upgrade never edits source files.
- `LICENSE` is never written, because changing a game's license text has legal effect. When the new template's license differs, an untouched `LICENSE` is listed for you to review.
- The new version decides what changed. After install, the old version hands its template to the new binary, which compares and writes the files and prints the report. If the old template is unknown, every owned part that differs from the new template is listed and none is written, and every docs topic is listed. The old template is unknown when `package.json` pinned a version that excludes the installed one before the upgrade, when the old version describes its template in a format the new one does not know, or when the template it sent came from a different version than the one installed and the old pins do not both name that version. A failed install on Windows can leave the new `@trioforge/crucible-cli` beside the old binary, and the pins still name the old version, so a retry keeps the old template.

The template's `.gitattributes` sets `* text=auto eol=lf`, so a Windows checkout gets LF files and the game's `vp check` passes. Files checked out before the game had it keep their line endings until they are checked out again.

### Upgrading from roblox-ts

Games made by `crucible create` 0.3.0 and earlier build with roblox-ts. The old version's binary runs the install and does not know the new toolchain, so the new version's binary moves the game onto Crucible's compiler when it refreshes the template. The report lists what it did under "Moved this game from roblox-ts to Crucible's compiler" and what is left under "Finish the move by hand".

- When `package.json` lists `roblox-ts` or `@rbxts/compiler-types`, the upgrade removes `roblox-ts`, `typescript`, and `@rbxts/compiler-types`, adds `@trioforge/compiler-types` at the new version beside `@trioforge/crucible-cli`, and installs again. Only those entries change, so the file keeps its formatting. When they cannot be edited in place, `package.json` is left alone and the report names the edit and the install to run. If the game's own scripts import `typescript`, add it back.
- It deletes the modules rbxtsc compiled into each place's `outDir`, the `.luau` and `.lua` files that start with `-- Compiled with roblox-ts`, and the folders that leaves empty. `crucible build` deletes only modules it compiled, so it never removes these. Every other file stays.
- An untouched `tsconfig.json` takes the new template, like any other template file. An edited one is never rewritten. The report lists each change from this table that it still needs, and `crucible build` fails with `TS5102`, `TS5108`, or `TS2688` until the changes are made. The report follows `extends` through relative paths and packages in `node_modules`, and names the file that holds each setting, so a setting a base tsconfig holds is listed with that base. A base it cannot find or parse is listed for you to check by hand.

| TypeScript 5 setting | TypeScript 7 change |
|---|---|
| `moduleResolution: "Node"` | Set `module` to `"preserve"` and `moduleResolution` to `"bundler"`. |
| `baseUrl` | Remove it, and add a `paths` entry for each source root. |
| `downlevelIteration` | Remove it. |
| `typeRoots` | Remove it. |
| `types` without `@trioforge/compiler-types` | Set `types` to `["@trioforge/compiler-types", "@rbxts/types"]`, and name any other type package in full. |
| no `rootDir` | Set `rootDir` to the source folder, such as `"src"`. |
| no `skipLibCheck` | Set `skipLibCheck` to `true`. |
| `plugins` | Remove it. The compiler applies Crucible's transform itself. |

The report asks for a missing `types`, `rootDir`, or `skipLibCheck` only when it read every tsconfig in the chain, because a base it cannot read can supply them, and it names the place's own tsconfig for each. It asks only in a game moving off roblox-ts, because a newer game chose its own settings. After the changes, run `npm run check`.

The 0.3.0 binary starts the upgrade, so one of its limits needs a step first. On Windows, also read [Upgrading from 0.5.0 or older on Windows](#upgrading-from-050-or-older-on-windows).

When every package is in `devDependencies` and `package.json` has no `dependencies` field, 0.3.0 exits 1 with "cannot update package.json automatically". Add `"dependencies": {}` to `package.json`, then run `npx crucible upgrade`. You can delete the empty field afterwards.

### Upgrading from 0.5.0 or older on Windows

The game's installed version starts the upgrade, and up to 0.5.0 its shim runs the binary inside `node_modules/@trioforge/crucible-cli-win32-x64`, the folder the install replaces. Windows cannot delete a running binary, so the result depends on the package manager:

| Package manager | `npx crucible upgrade` from 0.5.0 or older |
| --- | --- |
| npm | Succeeds. npm warns that it could not remove `node_modules/@trioforge/.crucible-cli-win32-x64-<random>`, which holds the old binary. The next `crucible` command deletes it. |
| pnpm with its default isolated layout | Succeeds. |
| pnpm 12.15 or later with `nodeLinker: hoisted` | Succeeds. pnpm warns that it left the old binary in `node_modules/@trioforge/crucible-cli-win32-x64_pacquet-stage_<numbers>`. The next `crucible` command deletes it. |
| pnpm 12.14 or earlier with `nodeLinker: hoisted` | Fails with `os error 5`. Run `npx crucible upgrade <version>` again. |
| Yarn 1 | Fails with `EPERM`. Run `yarn crucible upgrade <version>` again. |
| Yarn 2 or later | Fails with `EPERM` and removes `@trioforge/crucible-cli`. Run a copy of the binary, as below. |
| Bun | Succeeds. |

The failed pnpm or Yarn 1 install has already installed the new `@trioforge/crucible-cli`, whose shim runs the old binary from a copy, so the second run finishes the upgrade and still knows the old template. Do not run the install that the first run's message names. For pnpm it changes nothing, but `yarn install` and `pnpm install --force` put the old shim back, and the second run then fails the same way.

Otherwise, run a copy of the binary from outside `node_modules`. It works before a first run too. It hands off to the new version as usual. With Yarn 2 or later after a failed run, first delete `node_modules/.yarn-state.yml` and run `yarn install --no-immutable`, because a plain install does not put the removed shim back. In PowerShell:

```powershell
Copy-Item node_modules\@trioforge\crucible-cli-win32-x64\crucible.exe "$env:TEMP\crucible-old.exe"
& "$env:TEMP\crucible-old.exe" upgrade 0.5.1
```

In Git Bash:

```sh
cp node_modules/@trioforge/crucible-cli-win32-x64/crucible.exe "$TEMP/crucible-old.exe"
"$TEMP/crucible-old.exe" upgrade 0.5.1
```

In Command Prompt:

```console
copy node_modules\@trioforge\crucible-cli-win32-x64\crucible.exe %TEMP%\crucible-old.exe
%TEMP%\crucible-old.exe upgrade 0.5.1
```

### When install fails

A failed install restores `package.json` and exits 1. `node_modules` can still hold a mix of versions, so the message names the command that returns it to the restored pins:

| Package manager | Command |
| --- | --- |
| npm | `npm install` |
| pnpm | `pnpm install --force --no-frozen-lockfile` |
| Yarn 1 | `yarn install` |
| Yarn 2 or later | Delete `node_modules/.yarn-state.yml`, then `yarn install --no-immutable` |
| Bun | `bun install` |

pnpm and Yarn 2 keep their own record of `node_modules`, and after a failed install that record can describe packages that were never put in place, so a plain install reports nothing to do. Run the command, then upgrade again. If the restore itself fails, the message names the temporary file beside `package.json` that holds the original.

The new version's binary runs a second install when it moves the game off roblox-ts. When that install fails, it restores `package.json` to the new version with the roblox-ts packages still listed, prints the report of the template files it already refreshed, and exits 1. Run the command the message names, then run `npx crucible upgrade` again. Crucible is then already at the new version, so the upgrade only finishes the move. An upgrade started from 0.3.0 also prints that the template was not refreshed, because that binary cannot tell this failure from a failed refresh. The report above that line lists what was refreshed.

An interrupted install, such as a closed terminal, leaves `package.json` at the new version. Restore `package.json` from version control, run the install command, then upgrade again. Running `npx crucible upgrade <version>` straight away also finishes the install, but it can skip or only list template changes, because the old template is no longer known.

### When the template refresh fails

After a good install, the refresh can still fail: an installed package or the binary reports another version, or the new binary fails or does not finish within 120 seconds. The command exits 1, and `package.json` and `node_modules` stay at the new version. Running `npx crucible upgrade` again then reports the version as already installed and does not refresh the template. The message says the template was not refreshed. Fix its cause, then compare by hand. `npx crucible create <name> <empty-dir> --no-install` writes the new template. Diff its owned files against the game's and copy the changes the game wants.

Stdout is the report an agent acts on: the two versions, the refreshed files, the edited, deleted, and dropped files to handle by hand, a changed `LICENSE` to review, what moved the game off roblox-ts and what is left of that move, and a `npx crucible docs <topic>` line for each topic whose content changed. Install output and progress go to stderr. `npx crucible create <name> <empty-dir> --no-install` writes the new template for a manual merge.

## crucible whoami

`npx crucible whoami` checks the Open Cloud key and prints one JSON document. The key comes from `CRUCIBLE_OC_API_KEY` in the environment, or from `.env` in the current directory, as [Configure a key](oc.md#configure-a-key) describes. The command posts the key to `/api-keys/v1/introspect` and sends nothing else.

```console
npx crucible whoami
npx crucible whoami --for universes.create-place-version
```

```json
{"keySource":".env","name":"publish-key","enabled":true,"expired":false,"scopes":["universe-places:write"]}
```

`keySource` is `CRUCIBLE_OC_API_KEY` or `.env`. Each scope's `name` and `operations` flatten into strings such as `asset:read`. The document never contains the key.

`--for <operation>` takes an operation name and checks that the returned scopes contain the operation's declared scope names. A missing scope exits 3 and names it. An unknown operation exits 2 and lists suggestions. Resource restrictions still apply on Roblox, so the check does not prove access to a particular universe or place. A missing key, a disabled or expired key, and a rejected key exit 3. `--verbose` writes redacted request diagnostics to stderr. The exit codes are the [Open Cloud exit codes](oc.md#exit-codes).
