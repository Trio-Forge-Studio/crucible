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
	ProfileEvent,
	ProfileMessage,
	ProfileObserver,
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

A handle is a callable table, not a function. Pass `() => h()` to code that tests `typeof(x) == "function"`, such as Charm's `computed`, `listen` and `subscribe` and `useAtom` from `vide-charm`. The types do not catch it.

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
- `grant(product, apply)` owns Roblox's `ProcessReceipt` for that product. It takes a handle from `crucible:experience`, such as `Products.CoinPack`, or a product ID. A purchase is acknowledged only after its change is durably saved, and `afterGrant` effects run once, after that.
- A Charm atom or published value that a claim or a handle callback writes is not rolled back when its data change fails: when the callback throws, or returns a value the store rejects. A `grant` callback also runs again each time Roblox retries an unsaved purchase. Players can then see a reward their profile does not hold. Write what players see in `afterGrant`, or after a handle write returns true.
- `quarantined(userId)` and `clearQuarantined(userId, ids, action)` recover messages a receiver kept failing on.

## Profile events

`observe(listener)` reports what happens to each player's profile on this server, for logs and metrics. Like a claim, it is a field initializer of a server Subsystem, and it stops when that Subsystem ends. Unlike a claim, it returns nothing and changes nothing: the listener cannot change data or delay a save.

The store is declared in a shared module, as every store is:

```ts
// src/shared/profile.ts
import { profileStore } from "@trioforge/crucible";

export const PlayerData = profileStore("Game.Players", { template: { coins: 0 } });
```

A server Subsystem observes it:

```ts
// src/server/profile-log.ts
import { Subsystem } from "@trioforge/crucible";
import type { ProfileEvent } from "@trioforge/crucible/server";
import { PlayerData } from "../shared/profile";

function line(event: ProfileEvent): string {
	const who = `${event.player.Name} (${event.player.UserId}) in ${event.store}`;
	switch (event.kind) {
		case "writable":
			return `${who} writable after ${event.seconds} s, ${event.attempts} requests (${event.reason})`;
		case "save":
			return event.result.ok
				? `${who} saved revision ${event.saved} in ${event.seconds} s`
				: `${who} save of revision ${event.revision} failed: ${event.result.reason}`;
		case "lost":
			return `${who} lost (${event.reason}) at revision ${event.revision}, ${event.saved} saved`;
		case "released":
			return `${who} released (${event.reason}): ${event.result.ok ? "saved" : event.result.reason}`;
	}
}

export class ProfileLog extends Subsystem {
	private readonly events = PlayerData.observe((event) => print(line(event)));
}
```

Every event carries its `kind`, the `store` name, the `player` and the session's `revision`, which counts its changes from 0 as loaded. None carries the profile's data.

- `writable`: the profile turned writable, joined or resumed. `attempts` counts the load's lease requests and `seconds` the whole load.
- `save`: a save that carried changes finished, automatic or from `save(player)`. `result` is that write's outcome, as a `SaveResult`, and can differ from what `save()` returns: `save()` waits until its deadline, so a write the store refused can report `unavailable` while `save()` returns `timeout`. `saved` is the highest revision the store now holds, and `attempts` and `seconds` that write's requests and duration. A lease renewal or a `save` that carries nothing new reports nothing. A failed save keeps its changes, and the next save carries them again.
- `lost`: the session stopped being writable without a release. `"fenced"` means another server took the profile, and `"authority"` that this server could not confirm its lease in time. The player is kicked.
- `released`: the release's last write settled, after a leave, a handoff, a shutdown, or `"lost"`. `result` is ok when the store holds every revision.

Each session reports `writable` first and `released` last, once each, with any number of saves between and `lost` at most once before `released`. A save already in flight when the session is lost can finish after `lost`. A load that fails reports nothing.

Each listener sees every event in order. It runs the way a claim does: it must not yield or write data, and a listener that throws, yields or writes is reported once per Subsystem and event kind, across every store it observes, while the other listeners and the save carry on. Schedule slow work, such as an HTTP call, from the listener with `task.spawn`.

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

## Delete and replace data in migrations

A migration returns a `MigrationPatch`. Ordinary objects merge recursively, arrays and scalars replace, and omitted keys stay in place. A patch without operations merges as it did before these operations existed, with exceptions for table keys and cycles. A patch that uses a table as a key refuses the load, even when a later step overwrites that table.

`migrationDelete` removes a key at any object depth. `migrationReplace(value)` replaces that entire value, including an object or an empty map. Both helpers and the `MigrationPatch`, `MigrationDelete`, and `MigrationReplacement` types are exported from `@trioforge/crucible`. A step can return `migrationReplace(record)` to replace the entire record and delete omitted keys.

```ts
import { migrationDelete, migrationReplace, type Migration } from "@trioforge/crucible";

const renameDiscoveries: Migration = (old) => ({
	discoveries: migrationReplace({ renamed: (old.discoveries as Record<string, number> | undefined)?.legacy ?? 0 }),
	obsolete: migrationDelete,
});
```

Operations belong at object keys in a patch, outside arrays and replacement values. Replacement values are literal JSON. `migrationReplace(undefined)` throws, including when a missing field is cast to `Json`. The delete marker is frozen. Returning only `migrationDelete`, returning a cyclic patch, or placing an operation inside a literal refuses the load before the next step runs. The next step sees plain data with the operations applied. A step before the last can return objects deeper than the persistence limit of 28 levels if a later step flattens them. The last step cannot, because admission refuses anything deeper in the final record. Checking a patch takes three stack frames, so an earlier step's temporary chain can be three levels less deep than before this check existed; in Lune that is about 19,990 levels. A cycle in an earlier step is refused, where it used to overflow the stack. A cycle in the last step goes through schema admission, which checks declared fields first and names array positions from zero. The final record still passes schema admission, which fills missing required object fields from defaults and validates the result before it becomes writable. To remove a required field permanently, remove it from the template too. An operation never goes into saved data. Steps remain pure, must not yield, and receive private copies. No `onLoad` cleanup is needed for a renamed map or a removed key.
