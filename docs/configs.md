# Configs

Read Roblox experience configs on the server through typed declarations that never throw.

Import configs from `@trioforge/crucible/server`. They are server only, because `ConfigService` is. Crucible never sends a config to a client and never refreshes one on its own: the game decides when values change and what clients see.

```ts
import { config, configReady, playerConfig, refreshConfigs } from "@trioforge/crucible/server";
import type { Config, PlayerConfig } from "@trioforge/crucible/server";
```

## Declare a config

A declaration names the dashboard key, the value's type and a fallback. Declare each one as an exported top-level `const` in a server module that boot loads, with the type argument written out.

```ts
import { config, playerConfig } from "@trioforge/crucible/server";

export interface GlobalEvent {
	readonly name: string;
	readonly startsAt: number;
	readonly endsAt: number;
	readonly multipliers: { readonly coins: number; readonly xp: number };
}

export const NO_EVENT: GlobalEvent = { name: "", startsAt: 0, endsAt: 0, multipliers: { coins: 1, xp: 1 } };

export const GlobalEventConfig = config<GlobalEvent>("GlobalEvent", NO_EVENT);
export const WelcomeDiscount = playerConfig<number>("Shop.WelcomeDiscount", 0);
```

`config` and `playerConfig` are compiler macros. The transformer plugin checks the declaration and adds the type's descriptor. Without the plugin's config support, a declaration throws "Crucible configs need the @trioforge/crucible/transformer plugin" when its module loads.

A key is up to 64 bytes of dot-separated segments that start with a letter and hold letters, digits and underscores. `Crucible` is reserved as the first segment. A duplicate key, a fallback that does not match the type, more than 1,000 configs, or a declaration in a module that loads after boot throws when its module loads.

## Read a value

A `Config<T>` is a function of no arguments that returns the value of the global snapshot, which ignores targeting. A `PlayerConfig<T>` takes a player and returns that player's targeted value. Neither yields, throws or fetches.

```ts
import { config, playerConfig } from "@trioforge/crucible/server";

const CoinBonus = config<number>("Shop.CoinBonus", 1);
const WelcomeDiscount = playerConfig<number>("Shop.WelcomeDiscount", 0);

const bonus = CoinBonus();
for (const player of game.GetService("Players").GetPlayers()) print(player.Name, WelcomeDiscount(player) * bonus);
```

What a read returns:

- Before the global snapshot first loads, every read returns the fallback.
- A player read returns the global snapshot's value until that player's snapshot loads, then their targeted value. A player who left, or was never seen, reads the fallback.
- A key missing from the dashboard reads as the fallback, so deleting a key resets it.
- A value that does not match the type keeps the last good value, or the fallback if there is none. For a player's first targeted value, the last good value is what they read until then: the global snapshot's value, else the fallback. Crucible logs one warning per key and snapshot, naming the key, the path and the reason, never the value.
- A JSON field the type does not declare is kept in the returned table, though the type does not show it.
- Values are frozen copies.

Values are applied once at the first fetch, then change only inside `refreshConfigs`.

## Know when values are real

`configReady()` is true once the global snapshot has been applied. `configReady(player)` is true once that player's snapshot has been applied, and false before and after they leave. Check it where a fallback or an untargeted value must not be acted on.

```ts
import { config, configReady, refreshConfigs } from "@trioforge/crucible/server";

const CoinBonus = config<number>("Shop.CoinBonus", 1);

export function payout(coins: number): number {
	if (!configReady()) refreshConfigs();
	if (!configReady()) return coins;
	return coins * CoinBonus();
}
```

## Refresh

`refreshConfigs()` yields. It refreshes every snapshot that has newer values or failed, fetches every snapshot that has not loaded, and applies all of them in one pass. It returns true when every snapshot is current. It returns false when a request failed, a snapshot has not loaded, or the call ran past 10 seconds; values stay at their last good state, and a result that arrives late is dropped, so the next call fetches it again. Calls made while a refresh runs share its result.

```ts
import { refreshConfigs } from "@trioforge/crucible/server";

task.spawn(() => {
	while (true) {
		task.wait(60);
		refreshConfigs();
	}
});
```

An update published on the dashboard only marks a snapshot outdated. Nothing changes until the game calls `refreshConfigs`.

## Fetching and failures

The global snapshot is fetched when the server boots. Each player's snapshot is fetched when they join, only if the game declares a player config. A failed fetch retries after a random wait of up to 1, 2, 4 and so on seconds, at most 30. No boot or read waits for configs, and a ConfigService outage only leaves fallbacks in place. At most four ConfigService requests run at once, and player fetches wait their turn in join order.

In Studio, configs read an empty in-memory service, so every read returns its fallback. Set the `CrucibleLiveData` attribute on `ServerStorage` to `true` to use `ConfigService`, which then also sees values set with `ConfigService:SetTestingValue`.
