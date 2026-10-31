# Crucible CLI

Run `crucible` commands and read their output and exit codes.

`crucible` is the command line for Crucible games. Run it with `npx crucible <command>` from a project that has `crucible` in its `devDependencies`. `npx crucible docs cli` lists every command.

`crucible --help` lists the commands, and `crucible <command> --help` describes one. `crucible --version` prints the CLI version and the embedded Rojo version.

## Output and exit codes

Commands print text. The result goes to stdout, and progress and errors go to stderr. No command prompts for input. `crucible oc` prints one JSON document per command instead of text and has its own exit codes, described in `crucible docs oc`.

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
