import type { Readable } from "./state";

declare const storeKind: unique symbol;
declare const messageKind: unique symbol;
declare const claimKind: unique symbol;
declare const operationKind: unique symbol;
declare const serverOnlyKind: unique symbol;
export type Json = boolean | number | string | readonly Json[] | JsonRecord;
export interface JsonRecord {
	readonly [key: string]: Json | undefined;
}
export type Migration = (old: JsonRecord) => JsonRecord;
export interface ServerOnly<V> {
	readonly [serverOnlyKind]: V;
}

type Scalar = string | number | boolean;
export type ProfileData<T> =
	T extends ServerOnly<infer V>
		? ProfileData<V>
		: T extends Scalar | undefined
			? T
			: T extends ReadonlyMap<infer K, infer V>
				? ReadonlyMap<K, ProfileData<V>>
				: { readonly [K in keyof T]: ProfileData<T[K]> };
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
export type Profile<S> = S extends ProfileStore<infer T> ? ProfileData<T> : never;
export type ClientProfile<S> = S extends ProfileStore<infer T> ? ClientData<T> : never;

type IsUnion<V, C = V> = V extends unknown ? ([C] extends [V] ? false : true) : never;
export interface WriteHandle<V, A> extends Readable<V | A> {
	set(value: V): boolean;
	update(change: (current: V) => V): boolean;
}
export interface ArrayWrites<E> {
	insert(value: E, index?: number): boolean;
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
export type ReadHandle<V, A = never> = Readable<V | A> & ReadChildren<NonNullable<V>, A | Extract<V, undefined>>;

export type LoadReason = "join" | "resume";
export type ReleaseReason = "leave" | "handoff" | "shutdown";
export interface Purchase {
	readonly id: string;
	readonly productId: number;
	readonly currencySpent: number;
}
export type AfterGrant = (effect: () => void) => void;
export interface ProfileClaim {
	readonly [claimKind]: true;
}
export interface ProfileMessage<P> {
	readonly [messageKind]: { readonly payload: (value: P) => P };
}
export type OperationId = string & { readonly [operationKind]: true };
export interface QuarantinedMessage {
	readonly id: OperationId;
	readonly name: string;
	readonly reason: "payload" | "attempts";
	readonly attempts: number;
	readonly sentAt: number;
	readonly payload: Json;
}

export type SaveFailure = "unavailable" | "timeout" | "uncertain";
export type SaveResult = { readonly ok: true } | { readonly ok: false; readonly reason: SaveFailure };

export type StoreFailure =
	| "unavailable"
	| "timeout"
	| "uncertain"
	| "overloaded"
	| "full"
	| "expired"
	| "reused"
	| "closing";
export type StoreResult<T> =
	| { readonly ok: true; readonly value: T }
	| { readonly ok: false; readonly reason: StoreFailure };
export interface ProfileStore<T> {
	readonly [storeKind]: { readonly kind: "profiles"; readonly value: (value: T) => T };
	(player: Player): DataHandle<ProfileData<T>>;
	(): ReadHandle<ClientData<T>>;
	loaded(player: Player): boolean;
	loaded(): boolean;
	players(): readonly Player[];
	await(player: Player): boolean;
	save(player: Player, timeoutSeconds?: number): SaveResult;
	handoff(player: Player): SaveResult;
	resume(player: Player): boolean;
	inspect(userId: number): StoreResult<ProfileData<T> | undefined>;
	send<P>(userId: number, message: ProfileMessage<P>, payload: NoInfer<P>, id?: OperationId): StoreResult<undefined>;
	quarantined(userId: number): StoreResult<readonly QuarantinedMessage[]>;
	clearQuarantined(userId: number, ids: readonly OperationId[], action: "retry" | "discard"): StoreResult<number>;
	onLoad(transform: (player: Player, data: ProfileData<T>, reason: LoadReason) => ProfileData<T>): ProfileClaim;
	onRelease(transform: (player: Player, data: ProfileData<T>, reason: ReleaseReason) => ProfileData<T>): ProfileClaim;
	receive<P>(
		message: ProfileMessage<P>,
		apply: (player: Player, data: ProfileData<T>, payload: P) => ProfileData<T>,
	): ProfileClaim;
	grant(
		productId: number,
		apply: (player: Player, data: ProfileData<T>, purchase: Purchase, afterGrant: AfterGrant) => ProfileData<T>,
	): ProfileClaim;
}

export interface ProfileOptions<T> {
	readonly template: T;
	readonly migrations?: readonly Migration[];
}
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
export declare function profileStore<T extends object>(
	name: string,
	options: ProfileOptions<T> & ReservedKeyCheck<T>,
): ProfileStore<T>;
export declare function serverOnly<V>(value: V): ServerOnly<V>;
export declare function replicatedPaths<T>(store: ProfileStore<T>): readonly (readonly string[])[];
