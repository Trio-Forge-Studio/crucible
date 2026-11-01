import type { Migration, OperationId, StoreFailure, StoreResult } from "../data";
import { newOperationId } from "../internal/data/operation-id";
import { dataPort } from "../internal/data/port";

declare const recordKind: unique symbol;

export interface StoreOptions<T> {
	/**
	 * A new record's value, and the value of a missing required key on an object path from the root. It never fills an
	 * array element, a map value or an optional key of an existing record. Admitted and frozen once.
	 */
	readonly defaults: T;
	/** Append-only. Removing or reordering a step changes what every older record becomes. */
	readonly migrations?: readonly Migration[];
}

/**
 * A generic update's result. `replayed` is true when the operation id was already recorded for the key: `change` did
 * not run, and `value` is the current stored record (the defaults when the key holds no payload), which may include
 * later writers' changes. A marker stores no result, so the original one cannot be returned.
 */
export type UpdateResult<T> =
	| { readonly ok: true; readonly value: T; readonly replayed: boolean }
	| { readonly ok: false; readonly reason: StoreFailure };

/**
 * Keyed records with no session. Each call is one DataStore request that yields and may fail, so the surface is two
 * calls with result unions, not live handles.
 */
export interface DataStore<T> {
	readonly [recordKind]: { readonly kind: "records"; readonly value: (value: T) => T };

	/** Yields. The stored record, migrated and admitted, or undefined when the key has none. */
	read(key: string): StoreResult<T | undefined>;
	/**
	 * Yields. One atomic change; `change` may run more than once, so it must be pure. A replayed id returns the current
	 * record with `replayed: true`. A throw in `change` cancels the write and is rethrown here.
	 */
	update(key: string, change: (current: T) => T, id?: OperationId): UpdateResult<T>;
}

/**
 * Declares keyed records by their DataStore name. It must initialize an exported top-level const in a server module,
 * with `T` explicit. `T` is an object or a string-keyed record at its root.
 */
export function dataStore<T extends object>(_name: string, _options: StoreOptions<NoInfer<T>>): DataStore<T> {
	error("Crucible data needs the @trioforge/crucible/transformer plugin");
}

/** A fresh operation identity. Create it once per logical operation and reuse it for that operation's retries. */
export function operationId(): OperationId {
	return newOperationId(dataPort()) as OperationId;
}
