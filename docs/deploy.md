# Deploy

Check that `crucible.lock` records what an environment declares, then build and publish its places with `crucible deploy`.

`crucible deploy <env>` ships the game to one environment of `crucible.toml`. It writes place versions and nothing else: it never creates, adopts or updates a product, a pass, a badge or an experience setting. `crucible provision <env>` does that, and its lock change is committed before the game that uses it is deployed. So a release from a tag can never mint a product. `crucible publish`, which 0.6.0 replaced, prints a usage error that names `crucible deploy` and exits 2.

```console
npx crucible provision production
git add crucible.lock && git commit -m "chore(experience): provision production"
npx crucible deploy production --dry-run
npx crucible deploy production
```

## What a run does

Each step runs only when the ones before it passed, so a refused check leaves Roblox and the files as they were.

1. It reads `crucible.toml`, `crucible.lock` and `.crucible/journal/<env>.json`, and stops with exit 2 on everything `crucible plan` stops on, with `error.problems`. It also stops with exit 2 when the environment sets neither `place` nor `places`, and the message names the mapping to add.
2. It plans the environment offline, as `crucible plan <env>` does, and stops with exit 2 unless every product, pass and badge step is a `no-op`. A `create` is a declared key with no ID in the lock, an `adopt-check` an intent in the journal whose create the lock does not resolve, and an `update`, `retire` or `untrack` a declaration the lock does not record yet. The message names each step and `crucible provision <env>`, and `error.steps` lists them as the plan prints them. Experience settings the lock does not record yet do not stop deploy: it prints a warning on stderr that names `crucible provision <env>` and goes on, so a settings update Roblox refuses never blocks a release.
3. It reads the key, posts it to Roblox's key introspection endpoint, and stops with exit 3 when the key is disabled or expired or lacks `universe-places:write`, which `error.missingScopes` names. When the introspection fails or takes longer than 10 seconds, deploy prints why on stderr and goes on, and an upload that lacks the scope fails by itself. `--no-scope-check` skips this step. The check matches the scope by name only, because the introspection reply as Crucible reads it does not say which universes a scope covers.
4. It builds the selected places, as `crucible build --env <env> --place` does, so each handle of `crucible:experience` holds that environment's ID.
5. It uploads each place in byte order by name, and stops at the first failed upload.

`--dry-run` stops after step 4. `npx crucible docs cli` describes `--place`, the JSON document, the exit codes and the `next` command that resumes a partial deploy.

## One provisioner per environment

Run one `crucible provision` per environment at a time, and deploy only from a commit that holds the lock it wrote. The lock is a file, not a distributed lock, and nothing checks this rule. For staging that is a developer's machine. For production it is a maintainer, or one CI job in a concurrency group.

In CI, deploy from the tagged commit with a key that has `universe-places:write`. That key needs none of the scopes provision needs, because deploy never calls the operations that use them.
