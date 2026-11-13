import { Subsystem } from "../../subsystem";
import { readClaim } from "../construction-claim";
import { isLive, stateOf } from "../lifecycle";
import { atom, computed, startBatch } from "@rbxts/charm";
import { peek, settle } from "../reactive/store";
import { define, type ClassRecord, type TypeId } from "../registry";
import { Kind, type ObjectState } from "../state";
import { claimPublication } from "../sync/declarations";
import { admit as admitReplicated, costOf, type Cost } from "../sync/values";
import { bootClock } from "../boot";
import { installServerSide, profileDeclarations, type ProfileDeclaration } from "./declarations";
import { rootHandle, type HandleSource } from "./handles";
import type { MessageDecl } from "./inbox";
import {
	CALL_DEADLINE_SECONDS,
	LOAD_DEADLINE_SECONDS,
	OWNER_VIEWS_MAX_BYTES,
	OWNER_VIEWS_MAX_ENTRIES,
	OWNER_VIEWS_MAX_VISITS,
	SAVE_TIMEOUT_DEFAULT_SECONDS,
	SAVE_TIMEOUT_MAX_SECONDS,
} from "./limits";
import { messageOf } from "./messages";
import { getPath, type Segment } from "./paths";
import { dataPort, type DataPort } from "./port";
import { DataEngine, type LoadReason, type ProfileRuntime, type ReleaseReason } from "./profiles";
import { bindRecords, unbindRecords } from "./record-bindings";
import { declaredRecords, sealDeclarations } from "./record-declarations";
import type { AfterGrant, Purchase } from "./receipts";
import { assertOutsideCallback, insideDataCallback, type Session } from "./session";
import { shutdownData } from "./shutdown";
import { Waiters } from "./waiters";

type ClaimKind = "onLoad" | "onRelease" | "receive" | "grant";

interface Claim {
	readonly kind: ClaimKind;
	readonly owner: string;
	readonly callback: Callback;
	readonly message?: MessageDecl;
	readonly productId?: number;
	live: boolean;
}

/** Rule 127's three measures. */
interface Share {
	readonly bytes: number;
	readonly entries: number;
	readonly visits: number;
}

/** Server state for every profile store in this realm: one engine, one activation, one claim index (rules 6, 104). */
const server = {
	engine: undefined as DataEngine | undefined,
	owner: undefined as ObjectState | undefined,
	activated: false,
	activation: undefined as Waiters | undefined,
	disconnect: new Array<() => void>(),
	receivers: new Map<string, string>(),
	grants: new Map<number, string>(),
	stores: 0,
};

const bindings = new Map<ProfileDeclaration, ProfileBinding>();

function port(): DataPort {
	return server.engine?.port ?? dataPort();
}

function varuintSize(value: number): number {
	let size = 1;
	while (value >= 128) {
		value = math.floor(value / 128);
		size += 1;
	}
	return size;
}

/** A player's slot: one state holding the admitted value while writable, else undefined (rule 9). */
class Slot implements HandleSource {
	readonly node = atom<object | undefined>(undefined);
	/** Whether the node holds a value, cut off so a data write never reruns a reader of `loaded` (rule 9). */
	readonly live = computed(() => this.node() !== undefined);
	readonly client = false;
	readonly name: string;
	readonly root: object;

	constructor(
		private readonly binding: ProfileBinding,
		private readonly player: Player,
	) {
		this.name = binding.declaration.name;
		this.root = rootHandle(this, binding.declaration.persist);
	}

	read(path: readonly Segment[]) {
		return getPath(this.node() ?? this.binding.declaration.schema.defaults, path);
	}

	beforeRead() {
		this.binding.beforeRead(this.player);
	}

	write(change: (root: object) => unknown) {
		return this.binding.write(this.player, change);
	}
}

/** Rule 115: a player who is not present reads the template and writes nothing, and gets no slot. */
function inertSource(binding: ProfileBinding): HandleSource {
	return {
		name: binding.declaration.name,
		client: false,
		read(path) {
			return getPath(binding.declaration.schema.defaults, path);
		},
		beforeRead() {},
		write() {
			binding.guard(undefined);
			return false;
		},
	};
}

/** One profile store's server side: slots, the gate, claims and the owner's views (rules 6, 9 to 12, 115 to 127). */
class ProfileBinding {
	readonly slots = new Map<Player, Slot>();
	readonly playersNode = atom<readonly Player[]>(table.freeze(new Array<Player>()));
	readonly claims = new Array<Claim>();
	/** The players whose own claims are running now, and which kind (rule 123). */
	readonly claiming = new Map<Player, string>();
	runtime?: ProfileRuntime;
	share?: Share;
	private readonly costs = new Array<WeakMap<object, Cost>>();

	constructor(readonly declaration: ProfileDeclaration) {
		for (let i = 0; i < declaration.fields.size(); i++) this.costs.push(new WeakMap());
	}

	slot(player: Player): Slot | undefined {
		let slot = this.slots.get(player);
		if (slot === undefined && port().players.present(player)) {
			slot = new Slot(this, player);
			this.slots.set(player, slot);
		}
		return slot;
	}

	root(player: Player): unknown {
		return this.slot(player)?.root ?? rootHandle(inertSource(this), this.declaration.persist);
	}

	/** Rule 13's checks. */
	guard(session: Session | undefined) {
		session?.assertIdle();
		assertOutsideCallback();
		if (insideDataCallback()) error("a write inside a data callback; return the data instead", 0);
	}

	/** Rule 12: the one gate. Before activation there is no session, so a write returns false. */
	write(player: Player, change: (root: object) => unknown): boolean {
		const runtime = this.runtime;
		this.guard(runtime?.sessions.get(player));
		if (runtime === undefined) return false;
		return runtime.write(player, change as (current: object) => object);
	}

	beforeRead(player: Player) {
		const claim = this.claiming.get(player);
		if (claim !== undefined)
			error(
				`${this.declaration.name}: ${player.UserId}'s handles are read inside its ${claim} claim; read the claim's data argument`,
				3,
			);
	}

	/**
	 * The engine's slot hook: every committed value, a load's publish and a seal's undefined (rule 9). Its readers run
	 * after the engine's bookkeeping, and one that throws is reported.
	 */
	publish(player: Player, value: object | undefined) {
		const slot = value === undefined ? this.slots.get(player) : this.slot(player);
		if (slot === undefined) return;
		startBatch();
		const before = peek(slot.node) !== undefined;
		slot.node(value);
		if (before !== (value !== undefined)) this.refreshPlayers();
		settle(`${this.declaration.name} changed`);
	}

	refreshPlayers() {
		this.playersNode(table.freeze([...(this.runtime?.players() ?? [])]));
	}

	/** Rule 9: the node is written undefined first, so readers settle, then the slot is forgotten. */
	drop(player: Player) {
		const slot = this.slots.get(player);
		if (slot === undefined) return;
		startBatch();
		slot.node(undefined);
		this.slots.delete(player);
		this.refreshPlayers();
		settle(`${this.declaration.name} dropped a player`);
	}

	loaded(player: Player): boolean {
		const slot = this.slot(player);
		if (slot === undefined || slot.live() !== true) return false;
		return this.runtime?.writable(player) === true;
	}

	/**
	 * Rule 127: each changed public field through reactive's own admission, then the store's whole cost, with every
	 * frame entry's key and wrapper, against its share. `committed` unset admits every field.
	 */
	checkViews(value: object, committed: object | undefined) {
		const declaration = this.declaration;
		const record = value as Record<string, unknown>;
		const before = committed as Record<string, unknown> | undefined;
		const live = declaration.liveDeclaration.name;
		let bytes = varuintSize(live.size()) + live.size() + costOf(true).bytes;
		let entries = 1 + costOf(true).entries;
		let visits = 2 + costOf(true).visits;
		declaration.fields.forEach((field, index) => {
			const current = record[field.name];
			const cache = this.costs[index];
			let cost = typeIs(current, "table") && before !== undefined ? cache.get(current) : undefined;
			if (cost === undefined || !rawequal(current, before?.[field.name])) {
				const admitted = admitReplicated(field.declaration.plan, `${declaration.name}.${field.name}`, current);
				cost = costOf(admitted);
				if (typeIs(current, "table")) cache.set(current, cost);
			}
			const key = field.declaration.name;
			bytes += varuintSize(key.size()) + key.size() + cost.bytes;
			entries += 1 + cost.entries;
			visits += 2 + cost.visits;
		});
		const share = this.share;
		if (share === undefined) return;
		const over = (measure: string, cost: number, limit: number) =>
			error(
				`${declaration.name}: the owner's views would cost ${cost} ${measure}, over this store's share of ${limit} (${server.stores} profile stores)`,
				0,
			);
		if (bytes > share.bytes) over("bytes", bytes, share.bytes);
		if (entries > share.entries) over("entries", entries, share.entries);
		if (visits > share.visits) over("visits", visits, share.visits);
	}

	/** Rule 18: the load's view check, then the live onLoad claims in order, each admitted like an update. */
	onLoad(session: Session, value: object, reason: LoadReason): object {
		const name = this.declaration.name;
		const [fits, why] = pcall(() => this.checkViews(value, undefined));
		if (!fits) {
			server.engine!.diagnose(
				`views:${name}:${session.token}`,
				`${name}: ${session.userId}'s profile does not fit the owner's views: ${tostring(why)}`,
			);
			error(why, 0);
		}
		this.runClaims(session, "onLoad", (claim, data) => claim.callback(session.player, data, reason), true, reason);
		return session.value;
	}

	/** Rule 43: a throwing or yielding claim is skipped with its input kept, and the others still run. */
	onRelease(session: Session, _value: object, reason: ReleaseReason): object {
		this.runClaims(
			session,
			"onRelease",
			(claim, data) => claim.callback(session.player, data, reason),
			false,
			reason,
		);
		return session.value;
	}

	private runClaims(
		session: Session,
		kind: "onLoad" | "onRelease",
		call: (claim: Claim, data: object) => unknown,
		fatal: boolean,
		reason: string,
	) {
		const name = this.declaration.name;
		this.claiming.set(session.player, kind);
		for (const claim of this.claims) {
			if (claim.kind !== kind || !claim.live) continue;
			const current = session.value;
			const [ok, err] = pcall(() => {
				const result = session.transact(() =>
					session.callback((data: object) => call(claim, data), current, 0),
				);
				if (!rawequal(result, current)) session.commit(session.prepare(result));
			});
			if (ok) continue;
			server.engine!.diagnose(
				`${kind}:${name}:${session.token}:${claim.owner}`,
				fatal
					? `${name}: ${session.userId}'s onLoad claim of ${claim.owner} failed (${reason}); the load failed`
					: `${name}: ${session.userId}'s onRelease claim of ${claim.owner} was skipped (${reason})`,
			);
			if (fatal) {
				this.claiming.delete(session.player);
				error(err, 0);
			}
		}
		this.claiming.delete(session.player);
	}

	/** Rule 6: claims are field initializers of server Subsystems, frozen at activation. */
	claim(
		kind: ClaimKind,
		callback: unknown,
		extra?: { message?: MessageDecl; productId?: number; absent?: true },
	): object {
		const name = this.declaration.name;
		const owner = readClaim();
		if (owner === undefined || owner.record.kind !== Kind.Subsystem)
			error(`${name}.${kind} runs while a server Subsystem is constructed; make it a field initializer`, 3);
		if (server.activated) error(`${name}.${kind}: claims are frozen once the data runtime has started`, 3);
		if (!typeIs(callback, "function")) error(`${name}.${kind} takes a function`, 3);
		if (extra?.absent) return table.freeze({});
		const label = (owner.record as ClassRecord).id;
		const claim: Claim = {
			kind,
			owner: label,
			callback: callback as Callback,
			message: extra?.message,
			productId: extra?.productId,
			live: true,
		};
		if (claim.message !== undefined) {
			const other = server.receivers.get(claim.message.name);
			if (other !== undefined)
				error(
					`${claim.message.name} is received by ${other} and ${label}; one Subsystem receives each message`,
					3,
				);
			server.receivers.set(claim.message.name, label);
		}
		if (claim.productId !== undefined) {
			const other = server.grants.get(claim.productId);
			if (other !== undefined)
				error(
					`product ${claim.productId} is granted by ${other} and ${label}; one grant handles each product`,
					3,
				);
			server.grants.set(claim.productId, label);
		}
		this.claims.push(claim);
		(owner.subscriptions ??= []).push(() => this.retireClaim(claim));
		return table.freeze({});
	}

	private retireClaim(claim: Claim) {
		if (!claim.live) return;
		claim.live = false;
		const engine = server.engine;
		if (claim.message !== undefined) {
			server.receivers.delete(claim.message.name);
			this.runtime?.inbox.unreceive(claim.message.name);
		}
		if (claim.productId !== undefined) {
			server.grants.delete(claim.productId);
			engine?.receipts.unroute(claim.productId);
		}
	}

	/** Rule 104: receivers and grants from the live claims. */
	route(engine: DataEngine): boolean {
		const runtime = this.runtime!;
		let granted = false;
		for (const claim of this.claims) {
			if (!claim.live) continue;
			if (claim.kind === "receive")
				runtime.inbox.receive(
					claim.message!,
					(player, data, payload) => claim.callback(player, data, payload) as object,
				);
			else if (claim.kind === "grant") {
				granted = true;
				engine.receipts.route(claim.productId!, {
					runtime,
					grant: (player: Player, data: object, purchase: Purchase, afterGrant: AfterGrant) =>
						claim.callback(player, data, purchase, afterGrant) as object,
				});
			}
		}
		return granted;
	}
}

function bindingOf(declaration: ProfileDeclaration): ProfileBinding {
	let binding = bindings.get(declaration);
	if (binding === undefined) {
		binding = new ProfileBinding(declaration);
		bindings.set(declaration, binding);
	}
	return binding;
}

/** Rule 4: a yielding call before activation waits for it within its own deadline. */
function activated(deadline: number): boolean {
	while (!server.activated && bootClock() < deadline) {
		server.activation ??= new Waiters(port());
		server.activation.park(deadline);
	}
	return server.activated;
}

function saveResult(result: string) {
	return result === "ok" ? { ok: true } : { ok: false, reason: result };
}

function storeResult(result: { readonly kind: string; readonly value?: unknown }) {
	return result.kind === "ok" ? { ok: true, value: result.value } : { ok: false, reason: result.kind };
}

function player(name: string, member: string, value: unknown): Player {
	if (value === undefined) error(`${name}.${member} needs its player on the server`, 3);
	return value as Player;
}

/** Rule 13: send and clearQuarantined are data writes too. */
function outsideCallbacks() {
	assertOutsideCallback();
	if (insideDataCallback()) error("a write inside a data callback; return the data instead", 0);
}

function member(declaration: ProfileDeclaration, key: string): unknown {
	const binding = bindingOf(declaration);
	const name = declaration.name;
	switch (key) {
		case "loaded":
			return (_: unknown, who?: Player) => binding.loaded(player(name, "loaded", who));
		case "players":
			return () => binding.playersNode();
		case "await":
			return (_: unknown, who?: Player) => {
				const target = player(name, "await", who);
				if (!activated(bootClock() + LOAD_DEADLINE_SECONDS)) return false;
				return binding.runtime!.await(target);
			};
		case "save":
			return (_: unknown, who?: Player, timeoutSeconds?: number) => {
				const target = player(name, "save", who);
				const timeout = math.clamp(timeoutSeconds ?? SAVE_TIMEOUT_DEFAULT_SECONDS, 0, SAVE_TIMEOUT_MAX_SECONDS);
				outsideCallbacks();
				if (!activated(bootClock() + timeout)) return saveResult("unavailable");
				return saveResult(binding.runtime!.save(target, timeout));
			};
		case "handoff":
			return (_: unknown, who?: Player) => {
				const target = player(name, "handoff", who);
				binding.guard(binding.runtime?.sessions.get(target));
				if (binding.runtime === undefined) return saveResult("unavailable");
				return saveResult(binding.runtime.handoff(target));
			};
		case "resume":
			return (_: unknown, who?: Player) => {
				const target = player(name, "resume", who);
				binding.guard(binding.runtime?.sessions.get(target));
				if (!activated(bootClock() + LOAD_DEADLINE_SECONDS)) return false;
				return binding.runtime!.resume(target);
			};
		case "inspect":
			return (_: unknown, userId: number) => {
				if (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });
				return storeResult(binding.runtime!.inspect(userId));
			};
		case "send":
			return (_: unknown, userId: number, message: unknown, payload: unknown, id?: string) => {
				const decl = messageOf(message) ?? error(`${name}.send takes a profile message`, 2);
				outsideCallbacks();
				if (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });
				return storeResult(binding.runtime!.inbox.send(userId, decl, payload, id));
			};
		case "quarantined":
			return (_: unknown, userId: number) => {
				if (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });
				return storeResult(binding.runtime!.inbox.quarantined(userId));
			};
		case "clearQuarantined":
			return (_: unknown, userId: number, ids: readonly string[], action: "retry" | "discard") => {
				outsideCallbacks();
				if (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });
				return storeResult(binding.runtime!.inbox.clearQuarantined(userId, ids, action));
			};
		case "onLoad":
		case "onRelease":
			return (_: unknown, callback: unknown) => binding.claim(key, callback);
		case "receive":
			return (_: unknown, message: unknown, callback: unknown) =>
				binding.claim("receive", callback, {
					message: messageOf(message) ?? error(`${name}.receive takes a profile message`, 2),
				});
		case "grant":
			return (_: unknown, product: unknown, callback: unknown) => {
				// A retired product the environment never had sends no receipt, so its grant claims no product.
				if (typeIs(product, "table") && (product as { readonly absent?: unknown }).absent === true)
					return binding.claim("grant", callback, { absent: true });
				// Reading a handle's id throws, naming the product, when the build holds no ID for it.
				const productId = typeIs(product, "table") ? (product as { readonly id?: unknown }).id : product;
				if (!typeIs(productId, "number")) error(`${name}.grant takes a product handle or a product id`, 2);
				return binding.claim("grant", callback, { productId });
			};
	}
	error(`${name} has no member ${key}`, 2);
}

installServerSide({ root: (declaration, who) => bindingOf(declaration).root(who), member });

function ended(object: object): boolean {
	const s = stateOf(object);
	return s === undefined || !isLive(s);
}

/** Rule 107, and a failed boot's rollback, which unseals the registry (rule 3). */
function retire() {
	const engine = server.engine;
	for (const disconnect of server.disconnect) disconnect();
	server.disconnect.clear();
	if (!server.activated) {
		sealDeclarations(false);
		server.engine = undefined;
		server.owner = undefined;
		for (const [, binding] of bindings) binding.runtime = undefined;
		return;
	}
	unbindRecords();
	engine?.retire();
}

/**
 * Rules 3, 104, 125 and 127 at construction: the registry seals, each store gets its engine, the shares are divided
 * and every template must fit its share, and the views are published under this construction claim.
 */
function construct() {
	const state = readClaim()!;
	const declarations = profileDeclarations();
	sealDeclarations(true);
	(state.subscriptions ??= []).push(retire);
	if (declarations.size() === 0 && declaredRecords().size() === 0) return;
	const engine = new DataEngine(dataPort());
	server.engine = engine;
	server.owner = state;
	server.stores = declarations.size();
	const count = math.max(declarations.size(), 1);
	const share: Share = {
		bytes: math.floor(OWNER_VIEWS_MAX_BYTES / count),
		entries: math.floor(OWNER_VIEWS_MAX_ENTRIES / count),
		visits: math.floor(OWNER_VIEWS_MAX_VISITS / count),
	};
	for (const declaration of declarations) {
		const binding = bindingOf(declaration);
		binding.share = share;
		binding.checkViews(declaration.schema.defaults, undefined);
		binding.runtime = engine.profile(
			{ ...declaration.schema, check: (value, committed) => binding.checkViews(value, committed) },
			{
				onLoad: (session, value, reason) => binding.onLoad(session, value, reason),
				onRelease: (session, value, reason) => binding.onRelease(session, value, reason),
				publish: (who, value) => binding.publish(who, value),
				delivered: () => {},
			},
		);
		claimPublication(declaration.live, (who: Player) => {
			const slot = binding.slot(who);
			return slot !== undefined && slot.live() === true;
		});
		for (const field of declaration.fields)
			claimPublication(field.handle, (who: Player) => {
				const slot = binding.slot(who);
				const value = slot === undefined ? undefined : slot.node();
				return value === undefined ? undefined : (value as Record<string, unknown>)[field.name];
			});
	}
}

/** Rule 104: after one wait, a live generation means boot committed. */
function activate(owner: DataRuntime) {
	const engine = server.engine;
	if (engine === undefined) return;
	engine.port.wait(0);
	if (ended(owner)) return;
	server.activated = true;
	bindRecords(engine);
	let granted = false;
	for (const [, binding] of bindings) if (binding.runtime !== undefined && binding.route(engine)) granted = true;
	if (granted) engine.receipts.install(engine.port.marketplace());
	const players = engine.port.players;
	const live = new Array<ProfileBinding>();
	for (const [, binding] of bindings) if (binding.runtime !== undefined) live.push(binding);
	if (live.size() > 0) {
		const join = (who: Player) => {
			for (const binding of live) engine.port.spawn(() => binding.runtime!.load(who, "join"));
		};
		const leave = (who: Player) => {
			for (const binding of live) {
				const runtime = binding.runtime!;
				engine.port.spawn(() => {
					runtime.leave(who);
					while (runtime.sessions.get(who) !== undefined) runtime.changed.park();
					binding.drop(who);
				});
			}
		};
		server.disconnect.push(players.onAdded(join), players.onRemoving(leave));
		for (const binding of live) for (const [who] of binding.slots) if (!players.present(who)) binding.drop(who);
		for (const who of players.all()) join(who);
	}
	server.activation?.wake();
}

/** @internal The framework's data Subsystem (rules 50 to 56, 104, 107). No injection identity; nothing depends on it. */
class DataRuntime extends Subsystem {
	constructor() {
		super();
		construct();
	}

	protected start() {
		activate(this);
	}

	protected onShutdown() {
		if (server.engine !== undefined) shutdownData(server.engine, this);
	}
}

let installed = false;

/** @internal The server entry defines DataRuntime once per realm, after ServerSync (rule 104). */
export function installDataRuntime() {
	if (installed) return;
	installed = true;
	define(DataRuntime, { kind: "subsystem", id: "Crucible.DataRuntime" as TypeId, deps: [], provides: [] });
}
