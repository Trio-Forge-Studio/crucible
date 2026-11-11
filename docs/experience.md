# Experience

Declare developer products, game passes and badges in `crucible.toml`, and plan their provisioning offline with `crucible plan`.

`crucible plan` reads the declarations, `crucible.lock` and the provisioning journal, and prints what `crucible provision` would do in each environment. It makes no network call and writes nothing. `crucible provision` itself arrives in a later release, so today the lock and the journal exist only when you write them.

## Declare the experience

```toml
#:schema ./node_modules/@trioforge/crucible-cli/crucible.schema.json

[places.start]
project = "default.project.json"

[experience]
private_server_price = 0
voice_chat = false

[env.staging]
universe = 10772177432
place = 132306245523809

[env.production]
universe = 10769815992
version_type = "Published"
place = 116566609396165

[products.coin-pack]
name = "Coin pack"
description = "100 coins."
price = 49
icon = "assets/icons/coin-pack.png"

[passes.vip]
name = "VIP"
price = 299

[badges.first-delivery]
name = "First delivery"
description = "Deliver your first parcel."
icon = "assets/badges/first-delivery.png"

[env.staging.products.coin-pack]
price = 1

[env.staging.badges.first-delivery]
enabled = false
```

- `crucible = "0.6"` at the top names the oldest crucible that can read the file. An older crucible that knows the key stops with `this project needs crucible 0.6.0 or later` before it reads anything else, so a key a newer release added never reads as unknown. A release from before the key reports ``unknown field `crucible` `` instead.
- The `#:schema` line gives Taplo and the Even Better TOML extension completion and checking from the schema the CLI package ships.
- A key such as `coin-pack` uses lowercase letters, digits and `-`, and starts with a letter. Game code will reach it as `Products.CoinPack`, so two keys that become the same handle, such as `a-1b` and `a1b`, are an error.
- A product or a pass needs a `name` and a `price` in Robux. A badge needs a `name`, and `enabled` defaults to true. `description` defaults to empty. `icon` is a path relative to the project root.
- `[env.<name>.products.<key>]` and `[env.<name>.passes.<key>]` may change only `price`, and `[env.<name>.badges.<key>]` only `enabled`. An overlay for a key that is not declared is an error, so a typo cannot create a resource.
- Every table rejects keys it does not know. Errors name the file, the line and the key, such as `crucible.toml:24: products.coin-pack.price: a price of 0 does not take a product off sale. Set retired = true to take it off sale`.

## Retire, untrack and rename

Roblox cannot delete a product, a pass or a badge, so removing a table that the lock holds an ID for is an error until you say what should happen to the resource.

- `retired = true` takes a product or a pass off sale and disables a badge. A price of 0 is not off sale. Once the lock records the resource as retired, you can delete its table, and the lock keeps the ID.
- `untrack = true` removes the resource from the lock and leaves it unchanged on Roblox.
- `moved_from = "<old key>"` renames a key and keeps its ID. Without it, a rename reads as a removal and a create, and `crucible plan` names the keys that could be the new one.

## Read the plan

```console
npx crucible plan staging
npx crucible plan --json
```

Without an environment, `crucible plan` plans every environment in `crucible.toml`. Each step is one of these actions.

| Action | When | What provision does |
| --- | --- | --- |
| `create` | The lock has no ID for the key. | Lists the universe's resources and adopts the one with the same name, or creates one. |
| `adopt-check` | The journal holds an intent for the key and the lock has no ID. | Treats the earlier create's outcome as unknown and looks for the resource before anything else. |
| `update` | The name, description, price, `enabled`, icon or key differs from what the lock recorded, or a retired resource is declared again. | Updates the resource in place. |
| `retire` | The table sets `retired = true` and the lock records the resource as managed. | Takes it off sale or disables it. |
| `untrack` | The table sets `untrack = true` and the lock holds its ID. | Removes it from the lock. |
| `no-op` | Nothing differs. | Nothing. |

`changes` says why an update happens: `details`, `icon`, `restore` or `key`. The lock keeps a fingerprint of the fields it applied, not the fields, so the plan cannot say which detail changed. More than five badge creates in one plan add a warning, because Roblox makes five badges a day free in each experience and charges 100 Robux for each one after that.

```text
staging (universe 10772177432)
  update  experience settings                         10772177432  details
  create  product coin-pack     Products.CoinPack
  create  pass vip              Passes.Vip
  create  badge first-delivery  Badges.FirstDelivery
  3 create, 1 update.
  Provision adopts the one existing resource with a create's name instead of creating it.
```

`--json` prints one document, `{"environments": [...]}`, with one plan per environment. IDs are strings.

## When the plan fails

| Exit | Meaning |
| --- | --- |
| 0 | The plan printed. |
| 1 | A file could not be read, or stdout could not be written. |
| 2 | `crucible.toml`, `crucible.lock` or the journal is invalid, the environment is unknown, or the plan breaks a rule above. With `--json`, `error.problems` lists each problem with its `file`, `line` and `key`. |

## The lock

`crucible.lock` is committed TOML, by environment, then kind, then key. Each environment records the universe its IDs belong to. A missing lock is a first provision. A lock that fails to parse, or has a newer `version` than this crucible reads, stops the plan. It never reads as empty, because an empty lock would mean creating everything again.

```toml
version = 1

[env.staging]
universe = 10772177432

[env.staging.experience]
fingerprint = "sha256:..."

[env.staging.products.coin-pack]
id = 3301234567
fingerprint = "sha256:..."
icon = "sha256:..."
status = "managed"
```

### Start an environment afresh

An ID means nothing in another universe, so when `env.<name>.universe` in `crucible.toml` differs from the universe the lock records for that environment, the plan stops and names both. If the ID changed by mistake, set it back. If the environment now deploys to a new universe, start it afresh: delete `[env.<name>]` and every `[env.<name>.*]` table from `crucible.lock`. The next plan creates everything in the new universe, and the resources in the old one stay on Roblox, unmanaged.

The journal at `.crucible/journal/<env>.json` lists the creates provision has started and not recorded. `specs/provision` in the Crucible repository models how provision uses both, and states what TLC checks.
