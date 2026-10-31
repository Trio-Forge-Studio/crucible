# Open Cloud

Call Roblox Open Cloud with `crucible oc`, which prints one JSON document per command.

Coding agents and Node scripts can call all 299 API-key operations in the pinned Roblox spec, plus API-key introspection and public place-to-universe lookup. Four commands handle asset upload, place publishing, asset download, and Luau execution. In a game project, run them as `npx crucible oc <command>`.

Each `crucible oc` command writes one JSON document to stdout. Progress goes to stderr. `npx crucible oc --help` prints plain text. `npx crucible --version` prints the CLI version. `npx crucible oc --version` is a usage error that prints JSON and exits 2. Nothing prompts for input. `crucible oc --help` prints a short guide with the exit codes.

## Configure a key

For CI, set `CRUCIBLE_OC_API_KEY` in the command's environment. Keys never belong in command arguments.

Local profiles live in `%APPDATA%\crucible\config.toml` on Windows. On macOS, the file is `~/Library/Application Support/crucible/config.toml`. On Linux, the file is `$XDG_CONFIG_HOME/crucible/config.toml`, or `~/.config/crucible/config.toml`. `CRUCIBLE_CONFIG` selects another file.

```toml
[oc]
default_profile = "assets"

[oc.profiles.assets]
key_env = "ROBLOX_ASSETS_API_KEY"
group = 35

[oc.profiles.publish]
key_env = "ROBLOX_API_KEY"

[oc.profiles.luau]
key_file = "C:/Users/Admin/.secrets/roblox-luau.key"
```

Open Cloud settings live in the `[oc]` table. A profile requires exactly one of `key_env` and `key_file`. Its optional `group` or `user` sets the default asset creator.

The selection order is `--profile`, `CRUCIBLE_OC_PROFILE`, `CRUCIBLE_OC_API_KEY`, then `default_profile`. A missing or empty selected source fails with exit 3. `crucible oc` never falls through to another source or retries with another key. Public calls send no key. Auth errors include the configured profile names and the operation's declared scopes. Missing scope metadata appears as `requiredScopes: null` and `scopeMetadata: "unknown"`.

## Find and describe operations

Find operations with words from their name, path, scopes, or documentation.

```console
npx crucible oc find luau logs
npx crucible oc describe universes.create-place-version
```

`describe` returns parameters, body media, schema outlines, scopes, safety, retry policy, pagination, completion, stability, and overlay notes. Names remain attached to their operations across spec updates.

## Call an operation

Parameters use `name=value`. `assetId`, `asset_id`, and `asset-id` bind the same parameter. Repeat array parameters as `ids=1 ids=2`. Required parameters and value kinds are checked before sending. JSON bodies are checked for syntax, without schema validation.

```console
npx crucible oc call assets.get assetId=1234567 readMask=assetType,displayName
npx crucible oc call GET '/assets/v1/assets/1234567?readMask=displayName'
npx crucible oc call universes.get-place-universe placeId=1818
npx crucible oc call assets.list-versions assetId=1234567 --all --max-items 50
npx crucible oc call universes.create-place-version universeId=13058 placeId=1818 versionType=Published --body @build/warehouse.rbxl
npx crucible oc call assets.create --part request=@request.json --part 'fileContent=@sofa.fbx;type=model/fbx' --no-wait
```

`--body` accepts inline JSON, `@file`, or `@-` for stdin. Binary operations accept the file's bytes. Multipart operations use repeated `--part name=value` arguments. Use `@@` when a literal starts with `@`. A process can consume stdin once.

`--all` follows pagination and returns one page-shaped document. `--max-pages` defaults to 100 and retains the next token when the cap is reached. `--max-items` bounds its item count and retains the next token when it stops early. `crucible oc` keeps the endpoint's default page size and reduces a declared size when the remaining item limit needs it. If a default page is too large, the outcome table permits one smaller-page read. A server that ignores the requested size produces exit 2. An unsafe operation is never re-sent for this adjustment.

Long-running calls poll by default. `--no-wait` returns the first reply. A pending result includes a GET-only command in `error.resume`. GET readers wait for terminal state by default too.

## Check credentials

```console
npx crucible oc auth check
npx crucible oc auth check --for assets.get
npx crucible oc auth profiles
```

`call api-keys.introspect` and `auth check` use the same executor path. It supplies the selected key as the `apiKey` body field and rejects caller-supplied bodies. `auth check` posts the selected key to `/api-keys/v1/introspect`. It flattens each scope's `name` and `operations` into strings such as `asset:read`. `--for` checks that the returned list contains the operation's declared scope names. Resource restrictions still apply on Roblox. `auth profiles` lists sources and whether their keys are present, without printing their values.

## Upload an asset

```console
npx crucible oc asset upload assets/sofa.fbx --group 35 --description 'Model for Fragile Delivery.'
npx crucible oc asset upload --resume 9f1c
```

Upload sends one create request, then polls for the asset ID. It never repeats the create request. A 5xx, lost response, malformed success body, or missing operation ID exits 5 and asks you to check the group's Creations. Rejections and unknown outcomes have different exit codes.

`--resume` accepts an operation ID or `operations/<id>` and sends GET requests only. It needs no file or creator. The default poll deadline is 300 seconds. After acceptance, a deadline, poll failure, or invalid asset receipt exits 8 with the resume command. A GET-only resume retains the read failure codes.

The display name defaults to the file stem in title case. `--name` overrides it. `--type` accepts `model`, `decal`, `audio`, or `video`; the default comes from the file extension. Files must be nonempty regular files of at most 20,000,000 bytes. Supported extensions are FBX, glTF, GLB, RBXM, PNG, JPG, JPEG, BMP, TGA, OGG, MP3, WAV, FLAC, and MP4. Use `--user` for individual ownership, or configure a creator in the selected profile.

## Publish a place

```console
npx crucible oc --profile publish place publish build/warehouse.rbxl --universe 13058 --place 1818
```

Publishing sends an `application/octet-stream` body. It retries 409 and 5xx responses for at most four attempts, with 60 seconds between attempts by default. A later success resolves earlier uncertainty. If any attempt got a 5xx or lost its response and none later succeeded, the command exits 5, including a 503 followed by a 400. `--version-type saved` saves without publishing.

## Download an asset

```console
npx crucible oc asset download 1234567 --out build/sofa.rbxm
npx crucible oc asset download 1234567 --version 2 --out build/sofa.rbxm --overwrite
```

Asset delivery needs `legacy-asset:manage`. `crucible oc` fetches the returned URL with a separate client that never carries the API key. It accepts HTTPS URLs without credentials and follows at most five redirects. The download goes through a sibling temporary file, then a rename. Existing files require `--overwrite`. Failed transfers preserve the previous target. Downloads have a 30-second idle read timeout and no total duration cap. Upload and publish requests with file bodies get 600 seconds, while polls keep short timeouts.

## Run Luau

```console
npx crucible oc luau run scripts/probe.luau --universe 13058 --place 1818
npx crucible oc luau run --resume universes/13058/places/1818/versions/7/luau-execution-sessions/e1/tasks/t1
```

Use `-` instead of a file to read UTF-8 Luau from stdin. Scripts are limited to 4 MB. `--version` selects a place version. `--timeout` sets Roblox's execution timeout in seconds.

`crucible oc` creates once, polls to COMPLETE, FAILED, or CANCELLED, then reads up to 100 log pages. Results come from `output.results`. FAILED and CANCELLED exit 6 with logs. If reading logs fails or exceeds 100 pages, the result includes `logsComplete: false`, the read error, and a stderr warning. A COMPLETE task still exits 0. An accepted task with an unmatched path exits 5 and retains the path without a resume command. A resume requires exactly one matching GET reader. A missing task identity for unfinished work exits 5. Poll failures retain `crucible oc luau run --resume <task path>` in `error.resume`. Resume sends GETs only, waits for terminal state, reads logs, and exits 6 on FAILED or CANCELLED.

## Exit codes

| Exit | `error.kind` | Meaning | Next action |
| --- | --- | --- | --- |
| 0 | | Success. | Read the document. |
| 2 | `usage` | Invalid arguments, input, local file, or read-result limits. | Fix the input. |
| 3 | `auth` | Missing key, HTTP 401 or 403, or failed credential check. | Fix the selected key or profile. |
| 4 | `rejected` | Another 4xx response. | Fix the request before repeating it. |
| 5 | `outcome_unknown` | A write may have acted. | Check the remote resource before repeating it. |
| 6 | `remote_failure` | An operation or Luau task failed. | Inspect the error and logs. |
| 7 | `unavailable` | Unprocessed, unavailable, or unreadable response. | Retry or resume a read. |
| 8 | `pending` | Accepted work is unfinished or a later poll or receipt check failed. | Run `error.resume`. |

An unsafe request answered with 429, or one that never left the process, is sent once and exits 7. The caller decides whether to repeat it. Read and idempotent operations retry 429, 5xx, and transport failures up to three attempts. Their default delays are 1 and 4 seconds. `Retry-After` in seconds takes precedence up to 300 seconds. A larger value stops retries and appears in `error.retryAfter`. Place publishing uses its overlay policy.

Once an unsafe request gets a 2xx, every later failure maps to 6 for remote failure, 8 with GET-only recovery when the resource is known, or 5 with a Creations check when it is not. Repeating the original write is unsafe. GET-only resumes retain codes 3, 4, and 7. A classified failure keeps its exit code if stdout cannot be written.

The global options are `--profile`, `--deadline <seconds>`, `--poll-interval <seconds>`, `--retry-delay <seconds>`, and `--verbose`. The default poll interval is 2 seconds. `--retry-delay` changes delays, without making an unsafe operation retryable.

## Call from Node

Run this script from the project directory with `crucible` installed as a devDependency. The fixed command uses a shell to run the Windows `.cmd` shim as well as Unix executables.

```javascript
import { spawnSync } from "node:child_process";

const result = spawnSync("npx crucible oc find asset", {
	encoding: "utf8",
	env: process.env,
	shell: true,
});
if (result.error) throw result.error;
const document = JSON.parse(result.stdout);
if (result.status === 5) throw new Error(document.error.hint);
if (result.status !== 0) throw new Error(JSON.stringify(document.error));
console.log(JSON.stringify(document));
```
