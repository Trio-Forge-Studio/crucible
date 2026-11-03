declare const operationKind: unique symbol;

/** A value a save can hold: JSON, with absent keys for values that admit `undefined`. */
export type Json = boolean | number | string | readonly Json[] | JsonRecord;

/** A saved record: string keys to JSON. */
export interface JsonRecord {
	readonly [key: string]: Json | undefined;
}

/**
 * One schema step. It gets a private copy of the stored record, unknown keys included, as plain JSON, and returns a
 * patch: an object merges into the stored object, any other value replaces the stored one, and an omitted key is kept.
 * A step cannot delete a key. Steps are append-only and must be pure.
 */
export type Migration = (old: JsonRecord) => JsonRecord;

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
