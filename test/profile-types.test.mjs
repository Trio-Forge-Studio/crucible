import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import { typecheck } from "./lib/typecheck.mjs";

// Data rule 8's type cases (probes/types P1 to P18 and N1 to N30) over the real runtime package's declarations, as a
// game sees them. Every negative line carries `@ts-expect-error`, which TypeScript reports when the line compiles, so
// the run fails unless each one is rejected.
const repo = path.resolve(import.meta.dirname, "..");
const sample = path.join(repo, "examples/sample-game").replaceAll("\\", "/");
const dir = `${sample}/src/profile-types`;

const template = `import { profileStore, serverOnly, type Migration, type Profile } from "@trioforge/crucible";

interface Job {
	readonly coins: number;
	readonly tip?: string;
}
const normalizeV0: Migration = (old) => (typeIs(old.coins, "number") ? { currency: old.coins } : {});

export const PlayerData = profileStore("FD.Players", {
	template: {
		currency: 0,
		upgrades: {} as Record<string, number>,
		questProgress: [] as number[],
		title: undefined as string | undefined,
		settings: { vibration: true, volume: { Game: 10, Music: 10 } },
		career: { jobs: [] as Job[] },
		redeemedCodes: serverOnly({} as Record<string, number>),
		analytics: serverOnly({ playTime: 0, lastSeen: undefined as number | undefined }),
	},
	migrations: [normalizeV0],
});
export type PlayerData = Profile<typeof PlayerData>;
`;

const message = `import { profileMessage } from "@trioforge/crucible/server";

export const AdminEdit = profileMessage<{ readonly Command: string; readonly Values: readonly number[]; readonly By: string }>(
	"FD.AdminEdit",
);
`;

const positive = `import { effect, Subsystem, watch, profileStore, replicatedPaths, serverOnly } from "@trioforge/crucible";
import type { ClientProfile, DataHandle, Profile, ReadHandle } from "@trioforge/crucible";
import { operationId, type OperationId, type SaveResult, type StoreResult } from "@trioforge/crucible/server";
import { AdminEdit } from "./message";
import { PlayerData } from "./template";

declare const player: Player;

const coins: number = PlayerData(player).currency();
const nested: number = PlayerData(player).settings.volume.Game();
const whole: Profile<typeof PlayerData> = PlayerData(player)();
const id: string = "speed";
const speed: number | undefined = PlayerData(player).upgrades[id]();
PlayerData(player).upgrades[id].set(3);
PlayerData(player).upgrades.speed.update((n) => (n ?? 0) + 1);
PlayerData(player).upgrades[id].set(undefined);
PlayerData(player).questProgress.insert(10);
PlayerData(player).questProgress.insert(5, 0);
PlayerData(player).questProgress.remove(0);
const first: number | undefined = PlayerData(player).questProgress[0]();
PlayerData(player).questProgress[0].set(11);
const jobCoins: number | undefined = PlayerData(player).career.jobs[0].coins();
PlayerData(player).career.jobs[0].coins.update((c) => c + 1);
PlayerData(player).update((d) => ({ ...d, currency: d.currency + 5, settings: { ...d.settings, vibration: false } }));
export class CoinWatcher extends Subsystem {
	protected override awake() {
		watch(PlayerData(player).currency, (now, before) => print(\`\${before} -> \${now}\`));
		effect(() => print(PlayerData().currency()));
	}
}
const lastSeen: number | undefined = PlayerData(player).analytics.lastSeen();
PlayerData(player).analytics.lastSeen.set(undefined);
PlayerData(player).title.set("Mover");
PlayerData(player).title.set(undefined);
const codes: number | undefined = PlayerData(player).redeemedCodes.SPRING();
const clientCoins: number = PlayerData().currency();
const clientEntry: number | undefined = PlayerData().upgrades[id]();
const ready: boolean = PlayerData.loaded();
const view: ClientProfile<typeof PlayerData> = PlayerData()();
const coinsForVide: () => number = () => PlayerData().currency();
const saved: SaveResult = PlayerData.save(player, 5);
if (!saved.ok) print(saved.reason === "uncertain" ? "may have committed" : saved.reason);
const handed = PlayerData.handoff(player);
if (handed.ok) PlayerData.resume(player);
const live: boolean = PlayerData.loaded(player) && PlayerData.await(player);
for (const each of PlayerData.players()) print(PlayerData(each).currency());
function showServer(h: DataHandle<number>): boolean {
	return h.set(h() + 1);
}
function showClient(h: ReadHandle<number>): number {
	return h();
}
showServer(PlayerData(player).currency);
showClient(PlayerData().currency);
export const Explicit = profileStore<{ readonly n: number }>("Game.Explicit", { template: { n: 0 } });
const inspected = PlayerData.inspect(42);
if (inspected.ok && inspected.value !== undefined) print(inspected.value.currency);
export class BadgeService extends Subsystem {
	private readonly badge = PlayerData.grant(9001, (who, data, purchase, afterGrant) => {
		afterGrant(() => print(\`\${who.Name} bought \${purchase.productId}\`));
		return { ...data, currency: data.currency + 100 };
	});
	private readonly reasons = PlayerData.onRelease((_player, data, reason) =>
		reason === "shutdown" ? data : { ...data, currency: data.currency },
	);
	private readonly stamp = PlayerData.onLoad((_player, data, reason) => (reason === "resume" ? data : data));
	private readonly edits = PlayerData.receive(AdminEdit, (_player, data, edit) => ({
		...data,
		currency: data.currency + edit.Values.size(),
	}));
}
const opId: OperationId = operationId();
const sent: StoreResult<undefined> = PlayerData.send(42, AdminEdit, { Command: "give", Values: [1], By: "a" }, opId);
const quarantined = PlayerData.quarantined(42);
if (quarantined.ok) PlayerData.clearQuarantined(42, quarantined.value.map((item) => item.id), "retry");
export const Hidden = profileStore("Game.Hidden", {
	template: {
		coins: 0,
		secret: { a: serverOnly(0), b: { c: serverOnly("") } },
		mixed: { shown: 0, hidden: serverOnly(0) },
	},
});
type HiddenClientKeys = keyof ClientProfile<typeof Hidden>;
const hiddenClientKeys: [HiddenClientKeys] extends ["coins" | "mixed"]
	? ["coins" | "mixed"] extends [HiddenClientKeys]
		? true
		: false
	: false = true;
const mixedKeys: keyof ClientProfile<typeof Hidden>["mixed"] = "shown";
const hiddenServer: string = Hidden(player).secret.b.c();
Hidden(player).secret.a.set(1);
export const AllHidden = profileStore("Game.AllHidden", { template: { a: serverOnly(0), b: { c: serverOnly(0) } } });
const noClientKeys: [keyof ClientProfile<typeof AllHidden>] extends [never] ? true : false = true;
const paths: readonly (readonly string[])[] = replicatedPaths(PlayerData);
const firstSegment: string | undefined = paths[0]?.[0];

export { coins, nested, whole, speed, first, jobCoins, lastSeen, codes, clientCoins, clientEntry, ready, view, sent };
export { hiddenClientKeys, mixedKeys, hiddenServer, noClientKeys, firstSegment, coinsForVide, live };
`;

const negative = `import { profileStore, replicatedPaths, serverOnly, type ProfileStore } from "@trioforge/crucible";
import { profileMessage } from "@trioforge/crucible/server";
import { AdminEdit } from "./message";
import { PlayerData } from "./template";

declare const player: Player;

// @ts-expect-error N1 a client set
PlayerData().currency.set(1);
// @ts-expect-error N2 a client reading a serverOnly field
PlayerData().analytics.playTime();
// @ts-expect-error N3 a reserved handle name as a template key
profileStore("Game.Reserved", { template: { coins: 0, update: 0 } });
// @ts-expect-error N4 a wrong value type at set
PlayerData(player).currency.set("lots");
// @ts-expect-error N5a update returning a wrong field type at the root
PlayerData(player).update((d) => ({ ...d, currency: "lots" }));
// @ts-expect-error N5b update returning a missing field below the root
PlayerData(player).settings.update((s) => ({ vibration: s.vibration }));
// @ts-expect-error N6 update returning nothing
PlayerData(player).update((d) => {});
// @ts-expect-error N7 mutating a read
PlayerData(player).settings().vibration = false;
// @ts-expect-error N8 set(undefined) on a required field
PlayerData(player).currency.set(undefined);
// @ts-expect-error N9 set(undefined) on an array element
PlayerData(player).career.jobs[0].set(undefined);
// @ts-expect-error N10a a client update
PlayerData().update((d) => d);
// @ts-expect-error N10b a client insert
PlayerData().career.jobs.insert(undefined!);
// @ts-expect-error N11 a handle called with a value
PlayerData(player).currency(5);
// @ts-expect-error N12 a nested serverOnly field through the client root read
PlayerData()().redeemedCodes;
// @ts-expect-error N13 a reserved name in an array element's type
profileStore("Game.ReservedElement", { template: { log: [] as { remove: number }[] } });
// @ts-expect-error N14 a server member without its player
PlayerData.save();
declare const wide: ProfileStore<{ readonly a: number | string }>;
// @ts-expect-error N15 stores are invariant
const narrow: ProfileStore<{ readonly a: number }> = wide;
// @ts-expect-error N16 a wrong message payload
PlayerData.send(1, AdminEdit, { Command: 1, Values: [], By: "a" });
// @ts-expect-error N17 a grant returning undefined
PlayerData.grant(1, (_player, _data) => undefined);
// @ts-expect-error N18 a plain string as an OperationId
PlayerData.send(1, AdminEdit, { Command: "give", Values: [1], By: "a" }, "batch-7");
// @ts-expect-error N19 a migration returning a number
profileStore("Game.M", { template: { n: 0 }, migrations: [(old) => 5] });
PlayerData.onRelease((_player, data, reason) => {
	// @ts-expect-error N20 a release reason "join"
	if (reason === "join") return data;
	return data;
});
// @ts-expect-error N21 a key the declared type lacks
PlayerData(player)().rating;
// @ts-expect-error N22 a client reaching a serverOnly field's handle
PlayerData().redeemedCodes;
// @ts-expect-error N23 a template that does not fit an explicit type
profileStore<{ readonly coins: number }>("Game.Coins", { template: { coins: "0" } });
// @ts-expect-error N24 a serverOnly value written as the plain value
PlayerData(player).currency.set(serverOnly(1));
const wideMessage = profileMessage<{ readonly x: number | string }>("Game.Wide");
// @ts-expect-error N27a message invariance
const narrowMessage: typeof AdminEdit = wideMessage;
// @ts-expect-error N27b a string product id
PlayerData.grant("123", (_player, data) => data);
PlayerData.receive(AdminEdit, (_player, data, edit) => {
	// @ts-expect-error N27c a receiver's payload type
	const n: number = edit.Command;
	return data;
});
// @ts-expect-error N28 a quarantine action other than retry or discard
PlayerData.clearQuarantined(1, [], "delete");
const Vaulted = profileStore("Game.Vaulted", { template: { coins: 0, vault: { pin: serverOnly(0) } } });
// @ts-expect-error N29a a client reaching an object whose only field is serverOnly
Vaulted().vault;
// @ts-expect-error N29b the same through the root read
Vaulted()().vault;
// @ts-expect-error N30 a replicated path assigned to a string
const dotted: string = replicatedPaths(PlayerData)[0];

export { narrow, narrowMessage, dotted };
`;

test("rule 8 P1 to P18 and N1 to N30 type-check against the runtime package's declarations", () => {
	const files = new Map([
		[`${dir}/template.ts`, template],
		[`${dir}/message.ts`, message],
		[`${dir}/positive.ts`, positive],
		[`${dir}/negative.ts`, negative],
	]);
	const result = typecheck(files);
	assert.equal(result.status, 0, result.output);
	const directives = negative.match(/@ts-expect-error N\d+/g) ?? [];
	assert.equal(directives.length, 33, "every negative case is present");
});
