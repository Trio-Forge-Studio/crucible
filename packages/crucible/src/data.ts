import type { MigrationPatch, MigrationReplacement } from "./migrations";
import type { ProductHandle } from "./experience";
import { markServerOnly, replicatedPathsOf } from "./internal/data/declarations";

declare const operationKind: unique symbol;
declare const storeKind: unique symbol;
declare const messageKind: unique symbol;
declare const claimKind: unique symbol;
declare const observerKind: unique symbol;
declare const serverOnlyKind: unique symbol;

/** A value a save can hold: JSON, with absent keys for values that admit `undefined`. */
export type Json = boolean | number | string | readonly Json[] | JsonRecord;

/** A saved record: string keys to JSON. */
export interface JsonRecord {
	readonly [key: string]: Json | undefined;
}

/**
 * A record as a profile store keeps it: its JSON, and how many of the store's migrations it has been through. Version 0
 * runs every migration on it; the store's migration count runs none.
 */
export interface VersionedRecord {
	readonly record: JsonRecord;
	readonly version: number;
}

/**
 * One schema step. It gets a private copy of the stored record, unknown keys included, as plain JSON, and returns a
 * patch: an object merges into the stored object, any other value replaces the stored one, and an omitted key is kept.
 * Use migrationDelete to remove a key and migrationReplace to replace a value whole. Steps are append-only and pure.
 */
export type Migration = (old: JsonRecord) => MigrationPatch | MigrationReplacement<JsonRecord>;

/**
 * The identity of one logical operation, kept across its retries. Only `operationId()` makes one; a plain string does
 * not type-check. It carries its creation time, which bounds how long it deduplicates.
 */
export type OperationId = string & { readonly [operationKind]: true };

/** Why a store call did not complete. */
export type StoreFailure =
	| "unavailable"
	| "timeout"
	| "uncertain"
	| "overloaded"
	| "full"
	| "expired"
	| "reused"
	| "closing";

/** A store call's result: narrow `ok` before reading `value`. */
export type StoreResult<T> =
	| { readonly ok: true; readonly value: T }
	| { readonly ok: false; readonly reason: StoreFailure };

/**
 * A template field the owner's client never receives. The brand is phantom; `serverOnly(value)` also marks the value at
 * runtime, and the declaration checks that the two agree.
 */
export interface ServerOnly<V> {
	readonly [serverOnlyKind]: V;
}

type Scalar = string | number | boolean;

/** What the server reads and writes: the template's type with every `ServerOnly` unwrapped, deeply readonly. */
export type ProfileData<T> =
	T extends ServerOnly<infer V>
		? ProfileData<V>
		: T extends Scalar | undefined
			? T
			: T extends ReadonlyMap<infer K, infer V>
				? ReadonlyMap<K, ProfileData<V>>
				: { readonly [K in keyof T]: ProfileData<T[K]> };

type IsUnion<V, C = V> = V extends unknown ? ([C] extends [V] ? false : true) : never;

/** A `ServerOnly` field, or a plain object field whose every field is server-only in turn. */
type ServerOnlyField<V> = [V] extends [ServerOnly<unknown>]
	? true
	: [V] extends [Scalar | undefined]
		? false
		: [V] extends [ReadonlyMap<unknown, unknown> | readonly unknown[]]
			? false
			: true extends IsUnion<V>
				? false
				: string extends keyof V
					? false
					: [keyof V] extends [never]
						? false
						: false extends { [K in keyof V]-?: ServerOnlyField<NonNullable<V[K]>> }[keyof V]
							? false
							: true;

/**
 * What the owner's client reads: every `ServerOnly` field removed, and every plain object left with no field, at any
 * depth. Deeply readonly.
 */
export type ClientData<T> = T extends Scalar | undefined
	? T
	: T extends ReadonlyMap<infer K, infer V>
		? ReadonlyMap<K, ClientData<V>>
		: T extends readonly unknown[]
			? { readonly [K in keyof T]: ClientData<T[K]> }
			: {
					readonly [K in keyof T as ServerOnlyField<NonNullable<T[K]>> extends true ? never : K]: ClientData<
						T[K]
					>;
				};

/** A store's server data type, for helper signatures: `export type PlayerData = Profile<typeof PlayerData>`. */
export type Profile<S> = S extends ProfileStore<infer T> ? ProfileData<T> : never;
/** A store's client data type. */
export type ClientProfile<S> = S extends ProfileStore<infer T> ? ClientData<T> : never;

/**
 * A server handle's own members. `h()` reads, tracked and frozen; while the profile is not writable it returns the
 * template's value at the path. `set` and `update` are one write each through the store's gate and return its result.
 */
export interface WriteHandle<V, A> {
	(): V | A;
	set(value: V): boolean;
	update(change: (current: V) => V): boolean;
}

/** An array handle's writes. Indices are TypeScript's, from 0. */
export interface ArrayWrites<E> {
	/** Inserts at `index`, from 0 to the length, or appends without one. */
	insert(value: E, index?: number): boolean;
	/** Removes the element at `index`, from 0 to length - 1. */
	remove(index: number): boolean;
}

type DataChildren<V, A> = [V] extends [Scalar]
	? unknown
	: [V] extends [readonly (infer E)[]]
		? ArrayWrites<E> & { readonly [index: number]: DataHandle<E, undefined> }
		: true extends IsUnion<V>
			? unknown
			: [V] extends [ReadonlyMap<string, infer E>]
				? { readonly [key: string]: DataHandle<E | undefined, A> }
				: string extends keyof V
					? { readonly [key: string]: DataHandle<V[string & keyof V] | undefined, A> }
					: { readonly [K in keyof V]-?: DataHandle<V[K], A> };

/** A server handle at a position of type `V`: reads `V | A`, writes `V`, and reaches its children by key. */
export type DataHandle<V, A = never> = WriteHandle<V, A> & DataChildren<NonNullable<V>, A | Extract<V, undefined>>;

type ReadChildren<V, A> = [V] extends [Scalar]
	? unknown
	: [V] extends [readonly (infer E)[]]
		? { readonly [index: number]: ReadHandle<E, undefined> }
		: true extends IsUnion<V>
			? unknown
			: [V] extends [ReadonlyMap<string, infer E>]
				? { readonly [key: string]: ReadHandle<E | undefined, A> }
				: string extends keyof V
					? { readonly [key: string]: ReadHandle<V[string & keyof V] | undefined, A> }
					: { readonly [K in keyof V]-?: ReadHandle<V[K], A> };

/** A client handle: the same paths and reads as the server's, and no writes. */
export type ReadHandle<V, A = never> = (() => V | A) & ReadChildren<NonNullable<V>, A | Extract<V, undefined>>;

/** Why a profile loaded. */
export type LoadReason = "join" | "resume";
/** Why a profile is releasing. */
export type ReleaseReason = "leave" | "handoff" | "shutdown";

/** A developer product purchase, read from Roblox's receipt. */
export interface Purchase {
	readonly id: string;
	readonly productId: number;
	readonly currencySpent: number;
}

/**
 * A grant callback's fourth argument. It stages an effect that runs once, on a fresh thread, after the grant is durably
 * saved and before the purchase is acknowledged; never after a refused grant.
 */
export type AfterGrant = (effect: () => void) => void;

/** A field holding it is the claim. Not callable. */
export interface ProfileClaim {
	readonly [claimKind]: true;
}

/** A durable message to one player's profile, declared in a server module. Invariant in `P`. */
export interface ProfileMessage<P> {
	readonly [messageKind]: { readonly payload: (value: P) => P };
}

/** One quarantined message, for recovery tooling. The payload is the stored JSON, unknown keys included. */
export interface QuarantinedMessage {
	readonly id: OperationId;
	readonly name: string;
	readonly reason: "payload" | "attempts";
	readonly attempts: number;
	/** The id's creation time, in wall-clock seconds. */
	readonly sentAt: number;
	readonly payload: Json;
}

/** Why a save or handoff did not confirm. */
export type SaveFailure = "unavailable" | "timeout" | "uncertain";
/** A save or handoff result: narrow `ok` first. */
export type SaveResult = { readonly ok: true } | { readonly ok: false; readonly reason: SaveFailure };

/** A field holding it is the observer. Not callable. */
export interface ProfileObserver {
	readonly [observerKind]: true;
}

/**
 * Something that happened to one player's profile on this server, for logs and metrics. It never holds the profile's
 * data. `revision` counts the session's changes, from 0 as loaded, and `saved` is the highest revision a confirmed
 * save holds, 0 until one does. `attempts` counts the requests sent to the DataStore and `seconds` how long it took.
 */
export type ProfileEvent =
	| {
			/** The profile turned writable: its load's lease requests and how long the whole load took. */
			readonly kind: "writable";
			readonly store: string;
			readonly player: Player;
			readonly revision: number;
			readonly reason: LoadReason;
			readonly attempts: number;
			readonly seconds: number;
	  }
	| {
			/** A save carrying new revisions finished, automatic or from `save(player)`. */
			readonly kind: "save";
			readonly store: string;
			readonly player: Player;
			readonly revision: number;
			readonly result: SaveResult;
			readonly saved: number;
			readonly attempts: number;
			readonly seconds: number;
	  }
	| {
			/**
			 * The session stopped being writable without a release: another server took the profile ("fenced"), or this
			 * one could not confirm its lease in time ("authority"). Its release follows.
			 */
			readonly kind: "lost";
			readonly store: string;
			readonly player: Player;
			readonly revision: number;
			readonly reason: "fenced" | "authority";
			readonly saved: number;
	  }
	| {
			/** The release's final write settled. `result` is ok when it saved every revision. */
			readonly kind: "released";
			readonly store: string;
			readonly player: Player;
			readonly revision: number;
			readonly reason: ReleaseReason | "lost";
			readonly result: SaveResult;
			readonly saved: number;
			readonly attempts: number;
			readonly seconds: number;
	  };

/**
 * A profile store: one shared declaration, callable on both sides. The server calls it with a player for that player's
 * writable handle; the client calls it with nothing for the local player's read-only handle.
 */
export interface ProfileStore<T> {
	readonly [storeKind]: { readonly kind: "profiles"; readonly value: (value: T) => T };

	/** Server: the player's root handle, the same one while they are present. */
	(player: Player): DataHandle<ProfileData<T>>;
	/** Client: the local player's read-only root handle. */
	(): ReadHandle<ClientData<T>>;

	/** Server. Tracked: whether a write to the player's profile would be accepted now. */
	loaded(player: Player): boolean;
	/** Client. Tracked: whether the server has published this player's profile and the client holds all of it. */
	loaded(): boolean;
	/** Server. Tracked: the players whose profiles are writable; it changes only when one starts or stops being. */
	players(): readonly Player[];
	/** Server. Yields until the profile is writable (true), or its load fails, the player leaves or the server closes. */
	await(player: Player): boolean;
	/** Server. Yields until the current revision is durably saved, for a rare critical point. */
	save(player: Player, timeoutSeconds?: number): SaveResult;
	/** Server. Seals and durably releases the profile while the player stays, before a teleport. */
	handoff(player: Player): SaveResult;
	/** Server. After a failed teleport: waits for the handoff's release to settle, then loads again. */
	resume(player: Player): boolean;
	/** Server. Reads any user's saved data, with no lock and no write. */
	inspect(userId: number): StoreResult<ProfileData<T> | undefined>;
	/** Server. Appends a message to a user's inbox, online or not, and waits until it is durable. */
	send<P>(userId: number, message: ProfileMessage<P>, payload: NoInfer<P>, id?: OperationId): StoreResult<undefined>;
	/** Server. A user's quarantined messages, read with no lock. */
	quarantined(userId: number): StoreResult<readonly QuarantinedMessage[]>;
	/** Server. Moves listed quarantined messages back to the inbox ("retry") or drops them for good ("discard"). */
	clearQuarantined(userId: number, ids: readonly OperationId[], action: "retry" | "discard"): StoreResult<number>;

	/**
	 * Server, a Subsystem field initializer: calls `listener` with each of this store's profile events, in order. It runs
	 * like a claim, without yielding or writing data, and cannot change data or delay a save.
	 */
	observe(listener: (event: ProfileEvent) => void): ProfileObserver;

	/**
	 * Server claim, a Subsystem field initializer, at most one per store: brings in a player's data from a store Crucible
	 * did not write, once. It runs after the profile's lease is taken and before onLoad, and may yield. It gets the
	 * stored record, or undefined for a new player, and returns the record to start from, or undefined to keep the
	 * stored one. Its result goes through the store's migrations and admission. Once a save holds it, it never runs
	 * again for that player; a load that fails or ends first runs it again next time.
	 */
	importLegacy(
		read: (player: Player, stored: VersionedRecord | undefined) => VersionedRecord | undefined,
	): ProfileClaim;
	/** Server claim, a Subsystem field initializer: changes a profile as it loads, before anything reads it. */
	onLoad(transform: (player: Player, data: ProfileData<T>, reason: LoadReason) => ProfileData<T>): ProfileClaim;
	/** Server claim: changes a profile as it releases. A claim that throws is skipped. */
	onRelease(transform: (player: Player, data: ProfileData<T>, reason: ReleaseReason) => ProfileData<T>): ProfileClaim;
	/** Server claim: applies a message to its target's profile, once per message id. */
	receive<P>(
		message: ProfileMessage<P>,
		apply: (player: Player, data: ProfileData<T>, payload: P) => ProfileData<T>,
	): ProfileClaim;
	/** Server claim: grants a developer product. The purchase is acknowledged only after the change is saved. */
	grant(
		product: ProductHandle,
		apply: (player: Player, data: ProfileData<T>, purchase: Purchase, afterGrant: AfterGrant) => ProfileData<T>,
	): ProfileClaim;
	/** Server claim: grants a developer product by its Roblox ID. */
	grant(
		productId: number,
		apply: (player: Player, data: ProfileData<T>, purchase: Purchase, afterGrant: AfterGrant) => ProfileData<T>,
	): ProfileClaim;
}

/** A profile store's options. */
export interface ProfileOptions<T> {
	/**
	 * The store's type, by inference, and a new profile's value. A missing required key on an object path from the root
	 * takes its value from here. Admitted and frozen once, with every `serverOnly` mark unwrapped.
	 */
	readonly template: T;
	/** Append-only. Removing or reordering a step changes what every older record becomes. */
	readonly migrations?: readonly Migration[];
}

/** The handle member names. A template key with one of these names, at any depth, does not compile. */
export type ReservedName = "set" | "update" | "insert" | "remove";

type ReservedKeys<T> =
	T extends ServerOnly<infer V>
		? ReservedKeys<V>
		: T extends Scalar | undefined
			? never
			: T extends ReadonlyMap<unknown, infer V>
				? ReservedKeys<V>
				: T extends readonly (infer E)[]
					? ReservedKeys<E>
					: string extends keyof T
						? ReservedKeys<T[string & keyof T]>
						: (keyof T & ReservedName) | { [K in keyof T]-?: ReservedKeys<T[K]> }[keyof T];

type ReservedKeyCheck<T> = [ReservedKeys<T>] extends [never]
	? unknown
	: {
			readonly template: {
				readonly [K in ReservedKeys<T> as `"${K & string}" is a handle method; rename it`]: never;
			};
		};

/**
 * Declares a profile store by its DataStore name. It must initialize an exported top-level const in a shared module.
 * The type comes from `template`; annotate empty collections and optional fields with `as`.
 */
export function profileStore<T extends object>(
	_name: string,
	_options: ProfileOptions<T> & ReservedKeyCheck<T>,
): ProfileStore<T> {
	error("Crucible data was not compiled by the Crucible compiler; use crucible build");
}

/** Marks a template field server-only: the owner's client never receives it. */
export function serverOnly<V>(value: V): ServerOnly<V> {
	return markServerOnly(value) as ServerOnly<V>;
}

/**
 * The exact field paths the owner's client receives, each an array of segments: an object field's key, "[]" for an
 * array element and "*" for a map value. A key that is "[]" or "*", or starts with a backslash, gets one more backslash.
 * Sorted segment by segment, by bytes.
 */
export function replicatedPaths<T>(store: ProfileStore<T>): readonly (readonly string[])[] {
	return replicatedPathsOf(store);
}
