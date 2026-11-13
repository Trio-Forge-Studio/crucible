# Experience

Declare developer products, game passes and badges in `crucible.toml`, reach them from game code through typed handles, and plan their provisioning offline with `crucible plan`.

`crucible plan` reads the declarations, `crucible.lock` and the provisioning journal, and prints what `crucible provision` would do in each environment. It makes no network call and writes nothing. `crucible provision` applies the plan and writes the lock and the journal, as `npx crucible docs provision` describes.

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
- The `#:schema` line gives Taplo and the Even Better TOML extension completion and checking from the schema the CLI package ships. The schema checks what one table holds: unknown keys, types, key and name formats, prices of at least 1, a `name` on what is not retired and a `price` on a product or a pass that is neither retired nor untracked, and one of `place` or `places`. Only `crucible plan` and the other commands check the rules that span tables or need the running crucible:
  - a `crucible` version newer than the one running;
  - two keys that become the same handle;
  - two declared keys of one kind with one display name;
  - two environments with one universe;
  - a `moved_from` that names its own key, a key still declared, or a key another `moved_from` already names;
  - an overlay for a key that is not declared;
  - an environment that maps a place `[places]` does not declare, leaves one unmapped, or sets `place` when `start` is not the only place.
- A key such as `coin-pack` uses lowercase letters, digits and `-`, and starts with a letter. Game code will reach it as `Products.CoinPack`, so two keys that become the same handle, such as `a-1b` and `a1b`, are an error.
- Two products, two passes or two badges may not share a `name`. A declared key's name is taken whatever its status, retired and untracked included, because provision adopts an existing resource by its name and could not tell them apart. Only a retired key may leave out its `name`. A product and a pass may share one.
- Crucible compares display names byte for byte, so case, trailing spaces and Unicode forms make different names to it, though Roblox may treat them as one. A name that starts or ends with whitespace is an error.
- Each environment needs its own universe. Provision adopts a resource by its name inside a universe, so two environments on one universe would bind each other's resources, and one environment's overlays would overwrite the other's values.
- A product or a pass needs a `name` and a `price` in Robux. A badge needs a `name`, and `enabled` defaults to true. `description` defaults to empty. `icon` is a path relative to the project root.
- `[env.<name>.products.<key>]` and `[env.<name>.passes.<key>]` may change only `price`, and `[env.<name>.badges.<key>]` only `enabled`. An overlay for a key that is not declared is an error, so a typo cannot create a resource.
- Every table rejects keys it does not know. A file that does not parse fails with the file and the line, such as `crucible.toml:14: invalid type: string "49", expected u64`. A rule the tables break also names the key, such as `crucible.toml:24: products.coin-pack.price: a price of 0 does not take a product off sale. Set retired = true to take it off sale`.

## Use the handles in game code

Game code names a product, a pass or a badge by its key, through `crucible:experience`, and never writes a Roblox ID.

```ts
import { profileStore, Subsystem } from "@trioforge/crucible";
import { Badges, Passes, Products } from "crucible:experience";

const PlayerData = profileStore("Game.Shop", { template: { coins: 0 } });

export class Shop extends Subsystem {
	private readonly coinPack = PlayerData.grant(Products.CoinPack, (_player, data) => ({
		...data,
		coins: data.coins + 100,
	}));

	award(player: Player) {
		game.GetService("BadgeService").AwardBadge(player.UserId, Badges.FirstPickup.id);
	}

	offerVip(player: Player) {
		game.GetService("MarketplaceService").PromptGamePassPurchase(player, Passes.Vip.id);
	}
}
```

- `crucible build` writes `src/crucible-experience.d.ts`, which declares `crucible:experience` with `Products`, `Passes` and `Badges`. Each key becomes a handle named in PascalCase, so `coin-pack` is `Products.CoinPack`, typed `ProductHandle<"coin-pack">`. An untracked key has no handle. The file holds no ID, so it is the same in every environment: commit it. The build writes it when `crucible.toml` declares a product, a pass or a badge, or when the file exists, and rewrites it whenever its bytes differ from what `crucible.toml` gives, so do not edit it. It writes the file whole through a rename, and leaves it alone when nothing changed.
- `ProductHandle`, `RetiredProductHandle`, `PassHandle` and `BadgeHandle` come from `@trioforge/crucible`. A handle has the `key` and the `id` of its resource. A profile store's `grant` takes a product handle. Crucible has no call of its own to award a badge or check a pass: Roblox's calls, such as `BadgeService.AwardBadge`, `MarketplaceService.PromptGamePassPurchase` and `MarketplaceService.PromptProductPurchase`, take the handle's `id`.
- The build puts the handles in the place as a module next to the shallowest folder of the `outDir` that the Rojo project maps under `ReplicatedStorage`, so the server and the client both read it. A build fails when the project maps none.
- `crucible build --env <name>` binds each handle to the ID `crucible.lock` records for that environment. It fails when the lock holds no ID for a key that is neither retired nor untracked, and names `crucible provision <name>`, so a place for an environment never ships a product it cannot sell. It also fails when the lock records the environment in another universe. `crucible publish <env>` builds this way.
- `crucible build` without `--env` binds no ID, so the game compiles before anything is provisioned. Reading a handle's `id` in that place throws an error that names the key. A grant reads it when its Subsystem is made, so the server fails at boot rather than with ID 0.
- A retired product is off sale, but a purchase made before it retired can still send a receipt, so it keeps its handle, typed `RetiredProductHandle`. In an environment whose lock holds no ID for it, the product never existed, so its handle is absent: a grant of it claims nothing and does not throw, because no receipt can name it, and reading its `id` throws an error that names the key and the environment. A retired pass or badge with no ID is absent the same way. A key that is neither retired nor untracked and has no ID still fails `crucible build --env`.
- A place that grants any product must grant each product `Products` declares exactly once, and a retired product once or not at all, or the build fails with CRU039, which names the key and the files. A product has one grant call site, so to grant it differently by case, branch inside the grant's function, not around two grant calls. A place that grants none, such as a place where nothing is sold, leaves `ProcessReceipt` unset, and Roblox offers its purchases again in a server that handles them.

## Retire, untrack and rename

Roblox cannot delete a product, a pass or a badge, so removing a table that the lock holds an ID for is an error until you say what should happen to the resource.

- `retired = true` takes a product or a pass off sale and disables a badge. A price of 0 is not off sale. Once the lock records the resource as retired, you can delete its table, and the lock keeps the ID.
- `untrack = true` removes the resource from the lock and leaves it unchanged on Roblox. The key keeps its `name` while it is declared, so no other key adopts the resource. Keep that `name` equal to the name the resource has on Roblox: Crucible no longer reads the resource, so a changed `name` frees the old one, and a new key that takes it adopts the resource. When a new key needs the same name, give the new key another one. Once you remove the key from the file, the resource is like one made by hand: a new key with its name adopts it.
- `moved_from = "<old key>"` renames a key and keeps its ID. Without it, a rename reads as a removal and a create, and `crucible plan` names the keys that could be the new one.

## Read the plan

```console
npx crucible plan staging
npx crucible plan --json
```

Without an environment, `crucible plan` plans every environment in `crucible.toml`. Either way it reads every environment in `crucible.lock` and every journal, and stops on one that `crucible.toml` does not declare, as "Rename or remove an environment" says. Each step is one of these actions.

| Action | When | What provision does |
| --- | --- | --- |
| `create` | The lock has no ID for the key. | Lists the universe's resources and adopts the one with the same name, or creates one. |
| `adopt-check` | The journal holds an intent for the key, or for the key it moved from, and the lock has no ID. | Treats the earlier create's outcome as unknown and looks for the resource, under the declared name and the name the intent sent, before anything else. |
| `update` | The name, description, price, `enabled`, icon or key differs from what the lock recorded, or a retired resource is declared again. | Updates the resource in place. |
| `retire` | The table sets `retired = true` and the lock records the resource as managed. | Takes it off sale or disables it. |
| `untrack` | The table sets `untrack = true` and the lock holds its ID. | Removes it from the lock. |
| `no-op` | Nothing differs. | Nothing. |

`crucible provision` plans with what Roblox lists too, so its plan can also show `adopt`, with the ID of the one resource a `create` or an `adopt-check` found, and an `update` for a change someone made on Roblox. `changes` says why an update happens: `details`, `icon`, `restore` or `key`. The lock keeps a fingerprint of the fields it applied, not the fields, so the plan cannot say which detail changed. More than five badge creates in one plan add a warning, because Roblox makes five badges a day free in each experience and charges 100 Robux for each one after that.

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
| 2 | `crucible.toml`, `crucible.lock` or the journal is invalid, the environment is unknown, or the plan breaks a rule above. With `--json`, `error.problems` lists each rule the files break with its `file`, its `key` and, when one table holds the cause, its `line`. A file that does not parse, a newer format or an unknown environment leaves `problems` empty, and `error.message` says what failed. |

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

### Rename or remove an environment

Each environment's plan uses only that environment's lock tables and journal, so the IDs and intents of an environment `crucible.toml` no longer declares would never be planned. Renaming `[env.staging]` to `[env.stage]` would then plan a create for every resource staging already holds. So the plan stops while `crucible.lock` holds an `[env.<name>]` or `.crucible/journal/<name>.json` exists for an environment `crucible.toml` does not declare. The message names the environment, its universe and how many resources and intents it holds, and starts with any intent the lock does not resolve. Then do one of these:

- Declare the environment again.
- If you renamed it and the universe is the same, rename `[env.<old>]` and every `[env.<old>.*]` table in `crucible.lock` to the new name, and rename its journal file to match.
- If it is gone for good, delete `[env.<old>]` and every `[env.<old>.*]` table from `crucible.lock`, look in its universe for the resources any unresolved intent names, and delete its journal. Its resources stay on Roblox, unmanaged.

## The journal

`.crucible/journal/<env>.json` lists the creates provision has started and not recorded in the lock, with the universe it sent them to, the display name each one sent, and the IDs of that kind the run knew before it. Provision removes the file when it holds no intent.

```json
{
  "version": 1,
  "universe": 10772177432,
  "intents": [{ "kind": "badge", "key": "first-delivery", "name": "First delivery", "before": [2124000000000000] }]
}
```

Every intent shows in the plan, as `intent sent "<name>"` on the step of its key, or of the key that moved from it. When the lock holds the ID, the create landed, and the step adds `resolved by the lock`. Otherwise the step is an `adopt-check`. An intent no step can carry stops the plan and says what to do: its key is no longer declared, or is retired or untracked while the lock has no ID, so provision would never look for the resource. The plan also stops when a declared name is the name another key's unresolved intent sent, because that key's create may have landed under it, and when the journal names another universe than `crucible.toml`.

`specs/provision` in the Crucible repository models how provision uses the lock and the journal, and states what TLC checks.
