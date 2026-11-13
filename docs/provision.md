# Provision

Create, adopt and update the products, passes and badges `crucible.toml` declares, and record their IDs in `crucible.lock`.

`crucible provision <env>` makes the developer products, game passes and badges in one environment's universe match `crucible.toml`. It lists what the universe holds, plans with the same rules as `crucible plan`, prints the plan, and applies it without asking. It writes each ID Roblox assigns to `crucible.lock`, which you commit. Run `npx crucible docs experience` for the declarations, the plan and the lock.

```console
npx crucible provision staging --dry-run
npx crucible provision staging
git add crucible.lock && git commit -m "chore(experience): provision staging"
```

It needs the game's Open Cloud key, read from `CRUCIBLE_OC_API_KEY` or `.env` as `npx crucible docs oc` describes, with `developer-product:read`, `developer-product:write`, `game-pass:read`, `game-pass:write`, `legacy-universe.badge:manage-and-spend-robux` and `legacy-universe.badge:write`, plus `legacy-badge:manage` to change a badge's icon. Badges are listed through Roblox's public badge endpoint, which needs no key. Run one provision per environment at a time: the lock is a file, and nothing stops two machines from both creating a resource.

## What a run does

1. It reads `crucible.toml`, `crucible.lock` and `.crucible/journal/<env>.json`, and stops on everything `crucible plan` stops on.
2. It lists every product, pass and badge of the universe, to the last page. A list it cannot read whole stops the run, because a resource it did not see would be created again. It reads by ID each resource the lock names that a list left out.
3. It plans. A key with no ID in the lock is adopted when exactly one resource has its declared name, and created when none has. A key with an ID is updated when its name, description, price, sale state, `enabled` state or icon differs from the declaration, on Roblox or in the lock.
4. It prints the plan, then applies it: adopts first, then updates, retirements and untracks, then creates.

`--dry-run` stops after step 3 and writes nothing. `--json` prints one document, `{"plan": ..., "applied": [...], "dryRun": false}`, with an `error` object when the run failed. Without `--json`, the plan and each applied action go to stdout, and errors to stderr. Experience settings, such as `private_server_price`, show in the plan, and provision does not write them yet.

Each action in `applied` is `adopt`, `create`, `update`, `retire` or `untrack`, with the kind, the key and the ID. An `update` sends only the fields that differ, and the icon when its file changed since the lock recorded it or the lock records none. A retire takes a product or a pass off sale or disables a badge, and repeats when someone puts it back on sale on Roblox. An untrack removes the key from the lock and leaves the resource on Roblox.

## Creates and the journal

Roblox takes no idempotency key on a create and cannot delete a product, a pass or a badge, so a create sent twice makes a duplicate for good. Provision sends each create at most once and never sends one when its outcome is unknown.

Before a create, provision writes an intent to `.crucible/journal/<env>.json`: the key, the name it sends, and the IDs of that kind it knows of. It then sends the create, writes the ID to `crucible.lock`, and clears the intent. It writes each file to a temporary file in the same folder and renames it over the old one, so a crash leaves the old file or the new one. A crash between two steps leaves files the next run starts from:

| Crash after | The next run |
| --- | --- |
| the intent | finds nothing the create could have made, drops the intent, and creates. |
| the create | finds the resource by its name, or by an ID the intent did not know, and adopts it. |
| the lock | sees the ID in the lock and clears the intent. |

When Roblox refuses a create, with exit 3, 4 or 7, nothing was made: provision clears the intent, then stops, and the next run creates the resource. A crash between the refusal and that write leaves the intent, which the next run handles as a create whose answer was lost.

When a create's answer is lost, the run stops with exit 5 and keeps the intent. The next run looks for the resource under the declared name and any ID the intent did not know, whatever its name now is, so a create renamed on Roblox meanwhile is still found. While an intent is unresolved, no other intent is written, so every new ID belongs to it.

A badge is the exception. Roblox's public badge list can lag behind a create, so provision does not settle a badge's lost create from the list. It stops and names the badge: look for it in the experience's badges on Roblox, then run `crucible adopt` with its ID, or remove the intent from the journal if it is not there.

## When provision stops

| Stop | Fix |
| --- | --- |
| Two resources have a key's declared name. | Run `crucible adopt <env> <kind> <key> <id>` with the one that is the key's, or rename the other on Roblox. |
| One resource is a candidate of two keys. | Adopt it for the key it belongs to. |
| A badge's create may have landed. | Adopt the badge, or remove the intent if there is none. |
| The lock names an ID Roblox does not have. | If the resource is gone, delete its `[env.<env>.<kind>.<key>]` table from `crucible.lock`. Provision never replaces a resource on its own, because the lock may come from another universe. |
| A list leaves out an ID the lock names that Roblox returns by itself. | Run provision again later. Provision trusts the list to find a resource before it creates one. |
| Roblox rejects a badge create past the five it makes free each day, because provision offers to pay nothing. | Run provision again the next day. The rejected create leaves no intent, so that run creates the badge. The plan warns when more than five badges may need creating. |

The plan's stops change nothing. Exit codes follow `crucible oc`, plus exit 1 for a file that cannot be read or written:

| Exit | Meaning |
| --- | --- |
| 0 | The plan was applied, or printed with `--dry-run`. |
| 1 | A file could not be read or written. |
| 2 | A file is invalid, the environment is unknown, or the plan stops. `error.problems` lists each cause with its `file`, `key` and `line`. |
| 3 | The key is missing, or Roblox refused it. |
| 4 | Roblox rejected a request. |
| 5 | A create or an update may have landed. `error.resource` names it. Run provision again: it looks before it writes. |
| 7 | Roblox was unavailable. Nothing was left unknown. |

A failure after some actions keeps them: `applied` lists what was written, and the lock and the journal already hold it.

## crucible adopt

```console
npx crucible adopt staging product coin-pack 3301234567
```

`crucible adopt <env> <kind> <key> <id>` records an existing resource as a key's in `crucible.lock` and clears the key's intent. It lists the kind and reads the resource, then refuses unless the key is declared and managed, the lock holds no ID for it, no other key holds the ID, the resource carries the key's declared name or is new since the key's intent, and no other key without an ID would adopt it too. It writes the lock, then the journal. `--json` prints `{"applied": [...]}`, or an `error` with the exit codes above.

## Assumptions

- Roblox shows every product and pass that exists in their lists, once a create has answered or its answer was lost. Provision cannot tell when it does not.
- One provision runs per environment at a time.
- A public badge list that leaves out disabled badges is not confirmed. A badge the lock names that the list leaves out and Roblox returns disabled by itself is taken as it is.

`specs/provision` in the Crucible repository models the run in TLA+ and states what TLC checks, and `crates/iac/tests/conformance.rs` checks this code against the model's invariants with faults.
