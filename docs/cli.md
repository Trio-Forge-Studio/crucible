# Crucible CLI

Run `crucible` commands and read their output and exit codes.

`crucible` is the command line for Crucible games. Run it with `npx crucible <command>` from a project that has `crucible` in its `devDependencies`. `npx crucible docs cli` lists every command.

`crucible --help` lists the commands, and `crucible <command> --help` describes one. `crucible --version` prints the version.

## Output and exit codes

Commands print text. The result goes to stdout, and progress and errors go to stderr. No command prompts for input. `crucible oc` prints one JSON document per command instead of text and has its own exit codes, described in `crucible docs oc`.

| Exit | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The command failed. |
| 2 | The arguments are invalid. The usage error is on stderr. |

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
