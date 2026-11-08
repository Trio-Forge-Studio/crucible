# Profile stores

Read and write each player's saved profile through typed handles, and show the owner's client exactly its public part.

A profile store is one declaration in a shared module. The server calls it with a player to get that player's writable handle. The owner's client calls it with nothing to get a read-only handle on its own profile. Crucible loads, saves, locks and releases each profile, and keeps every field you mark `serverOnly` off the client's type and off the wire.

```ts
import { profileStore, replicatedPaths, serverOnly } from "@trioforge/crucible";
import type {
	ArrayWrites,
	ClientData,
	ClientProfile,
	DataHandle,
	Json,
	JsonRecord,
	Migration,
	Profile,
	ProfileData,
	ProfileOptions,
	ProfileStore,
	ReadHandle,
	ReservedName,
	ServerOnly,
	WriteHandle,
} from "@trioforge/crucible";
import { profileMessage } from "@trioforge/crucible/server";
import type {
	AfterGrant,
	LoadReason,
	ProfileClaim,
	ProfileMessage,
	Purchase,
	QuarantinedMessage,
	ReleaseReason,
	SaveFailure,
	SaveResult,
} from "@trioforge/crucible/server";
```

## Declare a store

Declare each store as an exported top-level `const` in a shared module that boot loads. The template is the schema: its type is the store's type, and its values are a new profile's values.

```ts
import { profileStore, serverOnly, type Profile } from "@trioforge/crucible";

interface Job {
	readonly coins: number;
	readonly tip?: string;
}

export const PlayerData = profileStore("FD.Players", {
	template: {
		currency: 0,
		upgrades: {} as Record<string, number>,
		questProgress: [] as number[],
		title: undefined as string | undefined,
		settings: { vibration: true, volume: 0.5 },
		career: { jobs: [] as Job[] },
		redeemedCodes: serverOnly({} as Record<string, number>),
	},
});

export type PlayerData = Profile<typeof PlayerData>;
```

`profileStore` is a compiler macro. Compile it with the Crucible compiler through `crucible build`. If the code was not compiled by it, a declaration throws "Crucible data was not compiled by the Crucible compiler; use crucible build" when its module loads.

- Literals widen: `0` is a `number`, `"Mover"` a `string`. Give an empty collection its type with `as`, such as `[] as string[]` or `{} as Record<string, number>`. An optional field is `undefined as V | undefined`, absent in a new profile. A literal union needs `as` too.
- The root is an object with declared fields. Vector3, Color3, CFrame, Instances and numbers with a brand other than Crucible's widths cannot be saved.
- `set`, `update`, `insert` and `remove` are the handle members, so no template key may use those names, at any depth.
- The template, its `serverOnly` defaults and its migrations ship to clients as code. They hold no player data, but keep secret defaults and server-only code out of the declaring module.

A store name is its DataStore name: up to 50 bytes of dot-separated segments that start with a letter and hold letters, digits and underscores, with `Crucible` reserved as the first segment. Store names are unique across profile stores and data stores. A realm holds at most four profile stores.

Each top-level field the client receives has its own replicated declaration, `Crucible.Data.<store>.<field>`, plus one lifecycle declaration, `Crucible.Live.<store>`. A view name is at most 64 bytes, so the longest public field name a store allows is 64 − 15 − the store name's length: 39 bytes for `FD.Players`. A public top-level field must also be one segment. A field that does not fit is a compile error; rename it, mark it `serverOnly`, or shorten the store name. A store with N public fields needs 1 + N of the realm's 256 replicated declarations, and its declaration throws, naming both counts, when the realm lacks room for all of them.

## Keep fields off the client

`serverOnly(value)` marks one template field as never reaching the owner's client. The server reads and writes it like any other field. An object whose every field is server-only leaves the client entirely, at any depth.

Marks sit only on fields along object paths from the root, never inside an array element, a map value, a union or another mark. The declaration checks that the marks in the template's value match the marks in its type, path by path, and throws when they differ.

`replicatedPaths(store)` lists every field path the client receives, each an array of segments: an object field's key, `"[]"` for an array element, `"*"` for a map value. A key that is `"[]"` or `"*"`, or starts with a backslash, gets one more backslash in front. A guard spec that asserts this list makes adding a field a decision: mark it `serverOnly`, or add its paths.

```ts
import { profileStore, replicatedPaths, serverOnly } from "@trioforge/crucible";

const Store = profileStore("Game.Guarded", {
	template: { coins: 0, stats: { wins: 0 }, secret: serverOnly("") },
});

const paths: readonly (readonly string[])[] = replicatedPaths(Store);
for (const path of paths) print(path.join(" / "));
```

## Read and write on the server

`PlayerData(player)` returns the player's root handle. Every field, map entry and array element has its own handle, typed by its position. A handle is called to read: the read is tracked, so effects and computeds rerun when its value changes, and the value is frozen.

```ts
import { profileStore } from "@trioforge/crucible";

const PlayerData = profileStore("Game.Wallet", {
	template: {
		currency: 0,
		upgrades: {} as Record<string, number>,
		questProgress: [] as number[],
		settings: { volume: 0.5 },
	},
});

export function reward(player: Player, amount: number, upgrade: string) {
	PlayerData(player).currency.update((c) => c + amount);
	PlayerData(player).upgrades[upgrade].update((level) => (level ?? 0) + 1);
	PlayerData(player).questProgress.insert(amount, 0);
	PlayerData(player).settings.volume.set(1);
	PlayerData(player).update((d) => ({ ...d, currency: d.currency - 5, settings: { ...d.settings, volume: 0 } }));
	return PlayerData(player).currency();
}
```

- `set(value)` and `update(change)` exist at every depth. Each is one write. A root `update` changes several fields in one write.
- Arrays add `insert(value, index?)` and `remove(index)`, with TypeScript's indices from 0.
- A map entry's `set(undefined)` removes it, and an optional field's `set(undefined)` clears it.
- A write below an absent map entry, array element or optional object throws; set the parent whole first. An index out of range throws.
- A write returns true when it was stored, or when its value equals the stored one. It returns false, and calls nothing, while the profile is not writable: before it loads, after it was released or handed off, or for a player who is not present.
- Handle callbacks must not yield or write data. Schedule that work after the write returns. A callback may write a Charm atom or a published value, and the effects the write reaches run after the callback returns.

While a profile is not writable, every read returns the template's value at that path. `PlayerData.loaded(player)` is the tracked test for whether the values are live. Read inside the write with `update` instead of reading first and writing later.

A key a newer build saved is kept in place. Spread a table to change it (`{ ...d, currency: 1 }`); a table rebuilt from scratch drops what it leaves out.

A handle is a callable table, not a function. Pass `() => h()` to code that tests `typeof(x) == "function"`.

## Store members

The members that act on the session stay on the store. They are server-only, except `loaded()` with no argument.

```ts
import { profileStore } from "@trioforge/crucible";

const PlayerData = profileStore("Game.Session", { template: { coins: 0 } });

export function beforeTeleport(player: Player): boolean {
	if (!PlayerData.await(player)) return false;
	const saved = PlayerData.save(player, 5);
	if (!saved.ok) warn(`save not confirmed: ${saved.reason}`);
	const handed = PlayerData.handoff(player);
	return handed.ok;
}

export function afterFailedTeleport(player: Player) {
	return PlayerData.resume(player);
}

export function report(userId: number) {
	const result = PlayerData.inspect(userId);
	if (result.ok && result.value !== undefined) print(result.value.coins);
	for (const player of PlayerData.players()) print(player.Name);
}
```

- `loaded(player)` and `players()` are tracked. `players()` changes only when a profile becomes or stops being writable.
- `await(player)` yields until the profile is writable and returns true, or returns false when the load fails, the player leaves or the server closes.
- `save(player, timeoutSeconds?)` is a barrier for a rare critical point. It yields until the current revision is durably saved: `ok`, or `unavailable`, `timeout` or `uncertain`.
- `handoff(player)` seals and releases the profile while the player stays, before a teleport. `resume(player)` loads it again after a failed teleport.
- `inspect(userId)` reads any user's saved data with no lock and no write.

## Claims

`onLoad`, `onRelease`, `receive` and `grant` are claims: field initializers of a server Subsystem. They run in construction order, get and return plain data, and must not yield or write data. A claim may write a Charm atom or a published value, and the effects the write reaches run after the claim returns. Reading the player's own handles inside an `onLoad` or `onRelease` claim throws; read the claim's data argument.

```ts
import { profileStore, Subsystem } from "@trioforge/crucible";
import { profileMessage } from "@trioforge/crucible/server";

const PlayerData = profileStore("Game.Claims", { template: { coins: 0, logins: 0 } });

export const Gift = profileMessage<{ readonly coins: number }>("Game.Gift");

export class RewardService extends Subsystem {
	private readonly login = PlayerData.onLoad((_player, data, reason) =>
		reason === "join" ? { ...data, logins: data.logins + 1 } : data,
	);
	private readonly leave = PlayerData.onRelease((_player, data) => data);
	private readonly gifts = PlayerData.receive(Gift, (_player, data, gift) => ({ ...data, coins: data.coins + gift.coins }));
	private readonly coins = PlayerData.grant(1234, (player, data, _purchase, afterGrant) => {
		afterGrant(() => print(`${player.Name} bought coins`));
		return { ...data, coins: data.coins + 100 };
	});

	sendGift(userId: number) {
		const sent = PlayerData.send(userId, Gift, { coins: 5 });
		if (!sent.ok) warn(`gift not queued: ${sent.reason}`);
	}
}
```

- A throwing `onLoad` claim fails the load and kicks the player. A throwing `onRelease` claim is skipped, keeping its input, and the other claims still run.
- `profileMessage<P>(name)` declares a durable message in a server module. `send(userId, message, payload, id?)` queues it for a player who is online, elsewhere or offline. Each message has one receiver per server.
- `grant(productId, apply)` owns Roblox's `ProcessReceipt` for that product. A purchase is acknowledged only after its change is durably saved, and `afterGrant` effects run once, after that.
- `quarantined(userId)` and `clearQuarantined(userId, ids, action)` recover messages a receiver kept failing on.

## The owner's client

On the client, `PlayerData()` returns the local player's read-only handle: the same paths as the server's, without `serverOnly` fields and without write members.

```ts
import { effect, profileStore, serverOnly, Subsystem } from "@trioforge/crucible";

const PlayerData = profileStore("Game.Hud", { template: { coins: 0, secret: serverOnly(0) } });

export class HudController extends Subsystem {
	protected override awake() {
		effect(() => print(`coins ${PlayerData().coins()}`));
		effect(() => print(PlayerData.loaded() ? "ready" : "loading"));
	}
}
```

- `PlayerData.loaded()` is true once the server has published this player's profile and the client holds all of it. It is false before the first sync, while the server loads, after a handoff or a lost session, and when sync fails.
- Before the profile arrives, reads return the template. After it arrives, reads return what the client last received, even while sync is failing.
- A write sends only the fields whose content changed. Fields changed in one write arrive in one update, and a write that touches only server-only fields or unknown keys sends nothing.

Each store's views are bounded so they cannot fill a sync frame. A write that would make a public field unsendable, such as a string over 49,152 bytes, throws at the write and stores nothing. All profile stores' views together take at most 65,536 bytes, 4,000 entries and 16,000 visits, divided evenly between the realm's stores.
