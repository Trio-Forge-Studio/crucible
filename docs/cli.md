# Crucible CLI

Run `crucible` commands and read their output and exit codes.

`crucible` is the command line for Crucible games. Run it with `npx crucible <command>` from a project that has `crucible` in its `devDependencies`. `npx crucible docs cli` lists every command.

`crucible --help` lists the commands, and `crucible <command> --help` describes one. `crucible --version` prints the CLI version and the embedded Rojo version.

## Output and exit codes

Commands print text. The result goes to stdout, and progress and errors go to stderr. No command prompts for input. `crucible oc` and `crucible publish` print one JSON document per command instead of text and have their own exit codes, described in `crucible docs oc`.

| Exit | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The command failed. |
| 2 | The arguments are invalid. The usage error is on stderr. |

## crucible build

Run `npx crucible build` from the game's root. It runs the project's roblox-ts once as `rbxtsc --type game`, then builds every place in `crucible.toml` to `build/<name>.rbxl` with the Rojo it embeds. `crucible --version` names that Rojo version. Projects need no separate Rojo install.

- roblox-ts resolves the way Node resolves `roblox-ts` from the game's root, through each parent `node_modules`. It runs with the `node` on `PATH`. Its diagnostics reach stderr with their file and line positions. When stderr is not a terminal, their colour codes are removed.
- `--place <name>` builds only that place. Repeat it to build several.
- `--output <file>` writes the place file elsewhere. It needs exactly one place.
- When the installed `@trioforge/crucible` transformer differs from the one recorded in `out/transformer.sha256`, `out` is deleted and the game compiles from clean.
- Exit 0 means every place built. Exit 1 means `crucible.toml` could not be read, or roblox-ts or Rojo failed. Exit 2 means a usage error or an invalid `crucible.toml`, reported before anything compiles.

`crucible.toml` lives at the game's root. It holds ids, never keys. Without the file, or without `[places]`, the game has one place, `start`, built from `default.project.json`.

```toml
[places.start]
project = "default.project.json"

[places.arena]
project = "arena.project.json"

[env.staging]
universe = 1234
profile = "publish"          # optional crucible oc profile
places = { start = 5678, arena = 5679 }

[env.production]
universe = 9999
version_type = "Published"   # default "Saved"
places = { start = 4321, arena = 4322 }
```

Every env must map every declared place, and only declared places. A game whose only place is `start` can write `place = 4321` in an env instead of `places`. Such a game can also omit `[places]`. Place and env names use letters, digits, `-` and `_`. Unknown keys are errors.

## crucible create

Run `npx crucible create my-game` to write a game into `./my-game` and install its dependencies. An optional path selects another directory, such as `npx crucible create my-game games/my-game`. The target must be absent or empty. Invalid names and occupied targets exit 2 without writing files. Names and path components cannot end in a dot or space or use Windows device names, including names with extensions such as `con.txt`. A target below an existing file also exits 2.

The game pins the CLI and runtime at the CLI version and includes the tested roblox-ts toolchain, Vite+, side entry scripts, transformer config, and a Rojo project. Template files use MIT-0. No separate Rojo or Rokit install is needed.

The command detects npm, pnpm, Yarn, or Bun from `npm_config_user_agent`, with npm as the default. Yarn projects include `.yarnrc.yml` with `nodeLinker: node-modules` for roblox-ts and Rojo. `--no-install` writes files without running install. Install failures exit 1 and leave the project available for a manual retry. Progress and install output go to stderr. Success prints the project path to stdout.

In the new directory, run `npx crucible dev` to compile on save and sync the game to Studio.

Run `npm run check` to lint, check formatting, compile, and build `build/start.rbxl`. `npm run build` builds the game, and `npm run fmt` fixes formatting. Use the equivalent script commands with your package manager. Read the generated `AGENTS.md` and `npx crucible docs cli` before developing the game.

## crucible dev

Run `npx crucible dev` from the game's root to compile on every save and sync one place to Studio. It starts the project's roblox-ts as `rbxtsc --watch --type game`. After the first compile, it serves the place with the Rojo it embeds. Connect the Rojo plugin in Studio to the port that Rojo prints.

```console
npx crucible dev
npx crucible dev --place arena --port 34873
```

- `--place <name>` serves that place from `crucible.toml`. The default is `start`.
- `--port <port>` sets the port. Without it, Rojo uses the project's `servePort`, or 34872.
- Every line from roblox-ts starts with `[rbxtsc]`, and every line from Rojo starts with `[rojo]`. All output goes to stderr. When stderr is not a terminal, colour codes are removed.
- roblox-ts resolves the same way as in `crucible build`. A changed `@trioforge/crucible` transformer makes the first compile start from clean.
- After the first compile, `crucible dev` writes `out/transformer.sha256`, even when that compile reports errors. The receipt records which transformer produced `out`, not whether the game compiles.
- Ctrl+C stops roblox-ts and Rojo, including any processes they started, and exits 0.
- If `crucible dev` itself is killed, roblox-ts, Rojo, and the processes roblox-ts started stop with it. On macOS and Linux, a process that roblox-ts starts in a new process group or session escapes this.
- If roblox-ts or Rojo stops on its own, `crucible dev` stops the other and exits 1. A port in use is the common cause.
- An unknown place or an invalid `crucible.toml` exits 2 before anything starts. A `crucible.toml` that cannot be read exits 1.

## crucible docs

Prints a guide as Markdown. Read the guide for a module before you write code that uses it.

```console
npx crucible docs
npx crucible docs logging
```

Without a topic, `crucible docs` prints one line per topic with its name and summary. With a topic, it prints that guide to stdout. An unknown topic exits 2 and lists the valid topics on stderr. The guides are compiled into the binary, so `crucible docs` needs no network.

## crucible oc

Calls Roblox Open Cloud. It uploads and downloads assets, publishes places, runs Luau, and calls any API-key operation in the Roblox spec. Each command prints one JSON document to stdout.

```console
npx crucible oc find data store entry
npx crucible oc --profile publish place publish build/game.rbxl --universe 13058 --place 1818
```

Run `npx crucible docs oc` for keys, commands, and exit codes. `npx crucible oc --help` prints a short form.

## crucible publish

`npx crucible publish production` builds the game, then uploads every place to the ids in `[env.production]` in `crucible.toml`. `--place <name>` selects a place and can repeat. Duplicate selections upload once. Places run in byte order by name. Credentials load before building. All selected places build before the first upload. The command stops at the first failed upload and does not roll back earlier uploads.

`--dry-run` builds the selected places and runs the same key and scope check as `crucible oc auth check --for universes.create-place-version`. It sends an introspection request and no upload requests. The check verifies that the key is enabled, unexpired, and has the declared operation scopes. It does not prove access to a particular universe or place.

`--profile <name>` overrides credentials. Selection order is `--profile`, `CRUCIBLE_OC_PROFILE`, `CRUCIBLE_OC_API_KEY`, the environment's `profile`, then the user config's `default_profile`. A selected missing or invalid credential fails without falling through. `--verbose` writes redacted request diagnostics to stderr. `version_type` defaults to `Saved`; set `Published` in the environment to publish the new versions.

Publish prints exactly one JSON document on stdout. Build progress and compiler diagnostics go to stderr. It uses the [Open Cloud exit codes](oc.md#exit-codes), including usage 2, auth 3, rejected 4, outcome_unknown 5, remote_failure 6, unavailable 7, and pending 8. Build failures are unavailable, exit 7. Missing `crucible.toml` is usage 2 with an example. Unknown environments or places are usage 2 with valid names.

Success, exit 0, includes each confirmed upload's version number. Ids are strings and version numbers are integers.

```json
{"environment":"production","dryRun":false,"published":[{"name":"arena","universeId":"9999","placeId":"4322","versionNumber":7,"versionType":"published"},{"name":"start","universeId":"9999","placeId":"4321","versionNumber":8,"versionType":"published"}],"failed":null,"notAttempted":[]}
```

An upload failure keeps the existing Open Cloud error fields and puts progress under `error.publish`. `failed` names the place whose upload failed. Its outcome can be unknown. `published` contains only confirmed receipts, and `notAttempted` lists the selected places after the failed place. The exit code classifies the failed upload independently of earlier successes. The `next` command selects the failed place and every unattempted place with `--place=<name>`. It preserves explicit `--profile` and `--verbose` flags. It does not copy environment variables. After a partial failure, never rerun the environment without `--place`, because that uploads confirmed places again. For `outcome_unknown`, check the failed place's versions before re-uploading it. Uploads follow the [place publishing retry policy](oc.md#publish-a-place).

```json
{"error":{"kind":"rejected","message":"HTTP 400","status":400,"detail":{},"publish":{"environment":"production","dryRun":false,"published":[{"name":"arena","universeId":"9999","placeId":"4322","versionNumber":7,"versionType":"published"}],"failed":"start","notAttempted":[],"next":"crucible publish production --place=start"}}}
```

Credential, build, and dry-run auth failures have the same progress object, with `failed` null and all selected places in `notAttempted`. Config and target validation errors return the usual `{"error":{"kind":"usage","message":"..."}}` before building.

A successful dry run exits 0 with no published receipts. `auth` is the existing auth-check result. All selected places remain unattempted.

```json
{"environment":"production","dryRun":true,"published":[],"failed":null,"notAttempted":["arena","start"],"auth":{"keySource":"profile publish","name":"publish-key","enabled":true,"expired":false,"scopes":["universe-places:write"]}}
```
