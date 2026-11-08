# Sample game

Read how the sample game uses a message, a replicated value, a profile store and a config together.

The sample lives in `examples/sample-game`. Its coins feature is a copy-ready reference. A player presses `E` to collect a coin. The server pays it with the current event multiplier, saves it in the player's profile, publishes the sum of every online player's coins, and tells the player what it gained. The player's screen shows its own coins, the total and the last gain.

| Part | Declaration | Side | File |
| --- | --- | --- | --- |
| Network message | `toServer` `Coins.Collect`, `toClient` `Coins.Collected` | Shared | `src/shared/messages.ts` |
| Replicated value | `replicated` `Coins.Total` | Shared | `src/shared/state.ts` |
| Profile store | `profileStore` `Sample.Players` | Shared | `src/shared/profile.ts` |
| Config | `config` `GlobalEvent` | Server | `src/server/systems/event.ts` |
| Server Subsystem | `Wallet` | Server | `src/server/systems/wallet.ts` |
| Client Subsystem | `Hud` | Client | `src/client/systems/hud.ts` |

The shared declarations load on both sides because the Subsystems import them. The sample's Rojo project maps `src/shared` to `ReplicatedStorage.TS`.

Each snippet below is a whole file from `examples/sample-game`, named above it. `test/docs.test.mjs` fails when a snippet differs from its file. Import the shared declarations on both sides instead of redeclaring them. A profile store is keyed by its name, so a client that declares a store under another name never receives what the server published.

## Declare the shared parts

`src/shared/messages.ts`

```ts
import { toClient, toServer } from "@trioforge/crucible";
import type { u8 } from "@trioforge/crucible";

export const Collect = toServer<{ readonly size: u8 }>("Coins.Collect");
export const Collected = toClient<{ readonly gained: number }>("Coins.Collected");
```

`src/shared/state.ts`

```ts
import { replicated } from "@trioforge/crucible";

export const TotalCoins = replicated<number>("Coins.Total", 0);
```

`src/shared/profile.ts`

```ts
import { profileStore, serverOnly } from "@trioforge/crucible";
import type { Migration } from "@trioforge/crucible";

const fromGold: Migration = (old) => ({ coins: typeIs(old.gold, "number") ? old.gold : 0 });

export const PlayerData = profileStore("Sample.Players", {
	template: { coins: 0, pickups: serverOnly(0) },
	migrations: [fromGold],
});
```

`coins` is public: the owner's client reads it. `pickups` is `serverOnly`: the server counts presses in it and no client receives it. The profile's schema version is the number of migrations. A profile saved before the migration held `gold`, and the migration turns it into `coins` when the profile loads.

## Pay on the server

`src/server/systems/event.ts`

```ts
import { config } from "@trioforge/crucible/server";

export interface EventConfig {
	readonly multiplier: number;
	readonly endsAt: number;
}

export const NO_EVENT: EventConfig = { multiplier: 1, endsAt: 0 };

export const GlobalEvent = config<EventConfig>("GlobalEvent", NO_EVENT);
```

`src/server/systems/wallet.ts`

```ts
import { computed } from "@rbxts/charm";
import { effect, Logging, receive, Subsystem } from "@trioforge/crucible";
import { Network, publish } from "@trioforge/crucible/server";
import { Collect, Collected } from "../../shared/messages";
import { PlayerData } from "../../shared/profile";
import { TotalCoins } from "../../shared/state";
import { GlobalEvent } from "./event";

const MAX_PICKUP = 5;
const log = Logging.getLogger("Sample.Wallet");

export class Wallet extends Subsystem {
	private readonly total = publish(TotalCoins);
	private readonly multiplier = computed(() => {
		const event = GlobalEvent();
		const live = event.endsAt === 0 || event.endsAt > os.time();
		return live ? event.multiplier : 1;
	});

	private readonly collect = receive(Collect, (player, { size }) => {
		if (size < 1 || size > MAX_PICKUP) return;
		const gained = size * this.multiplier();
		const wallet = PlayerData(player);
		if (!wallet.coins.update((coins) => coins + gained)) return;
		wallet.pickups.update((pickups) => pickups + 1);
		Network.send(player, Collected, { gained });
		log.info("{player} collected {gained}", { player: player.Name, gained });
	});

	protected override awake() {
		effect(() => {
			const multiplier = this.multiplier();
			log.info("coin multiplier {multiplier}", { multiplier });
		});
		effect(() => {
			let sum = 0;
			for (const player of PlayerData.players()) sum += PlayerData(player).coins();
			this.total(sum);
		});
	}
}
```

- The `u8` payload type bounds a pickup to 0 to 255 on the wire. The guard narrows it to 1 to 5, because the client chose the number.
- A write returns `false` while the profile is not writable, so a press that arrives before the profile loads pays nothing and is not counted.
- The multiplier is a Charm `computed` of the config, read by calling it. It is derived, so no effect writes it. The first effect only logs it, and reruns when `refreshConfigs` applies a value that changes it. A press after the refresh pays the new multiplier.
- The total effect reads every online profile, so it reruns when any of them changes. `PlayerData.players()` lists only players whose profile is writable.

## Show it on the client

`src/client/systems/hud.ts`

```ts
import { atom } from "@rbxts/charm";
import { effect, Logging, receive, Subsystem } from "@trioforge/crucible";
import { Network } from "@trioforge/crucible/client";
import { Collect, Collected } from "../../shared/messages";
import { PlayerData } from "../../shared/profile";
import { TotalCoins } from "../../shared/state";

const log = Logging.getLogger("Sample.Hud");
const Players = game.GetService("Players");
const UserInputService = game.GetService("UserInputService");

export class Hud extends Subsystem {
	private readonly lastGain = atom(0);

	private readonly collected = receive(Collected, ({ gained }) => {
		this.lastGain(gained);
	});

	protected override start() {
		const screen = new Instance("ScreenGui");
		screen.Name = "CoinHud";
		const label = new Instance("TextLabel");
		label.Name = "Coins";
		label.Size = new UDim2(0, 480, 0, 60);
		label.Position = new UDim2(0, 20, 0, 20);
		label.TextSize = 28;
		label.Parent = screen;
		screen.Parent = Players.LocalPlayer.WaitForChild("PlayerGui");
		effect(() => {
			const coins = PlayerData.loaded() ? PlayerData().coins() : 0;
			label.Text = `Coins: ${coins}   Total: ${TotalCoins()}   Last: +${this.lastGain()}`;
			log.info("hud {text}", { text: label.Text });
		});
		UserInputService.InputBegan.Connect((input, processed) => {
			if (!processed && input.KeyCode === Enum.KeyCode.E) Network.send(Collect, { size: 1 });
		});
	}
}
```

- `PlayerData()` on the client is read-only and holds only the public fields.
- The last gain is a Charm atom, read by calling it and written by calling it with the new value.
- The screen builds in `start`, which may yield, because `awake` must not.

## Run it

`pnpm check` builds the sample and runs `test/sample.luau` over two-realm fakes: one server and two clients, a shared data backend and a fake ConfigService. Each spec asserts what a client observes and what the data backend committed. The fakes do not prove Studio behaviour. `probes/sample/run.mjs` launches Roblox Studio on the built sample, starts a play test and presses `E` through the MCP keyboard tool. Studio runs the sample on the in-memory data and config ports. Set the `CrucibleLiveData` attribute on `ServerStorage` to `true` to use the real DataStore and ConfigService, and set the `GlobalEvent` config in the dashboard to change the multiplier.
