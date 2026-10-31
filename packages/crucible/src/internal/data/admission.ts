/** @internal What the durability engine knows of a profile schema. */
export interface Schema {
	readonly name: string;
	/** `migrations.size()` (rule 74). */
	readonly version: number;
	readonly defaults: object;
}

/** @internal */
export type Loaded =
	| { readonly kind: "ok"; readonly value: object; readonly migrated: boolean }
	| { readonly kind: "unreadable"; readonly reason: string }
	| { readonly kind: "newer"; readonly version: number };

/**
 * @internal The load seam: an identity on stored values until unit D3 adds migrations and admission (rules 74 to 77).
 * An unopened envelope takes the defaults (rule 18).
 */
export function loadRecord(schema: Schema, payload: object | undefined, version: number): Loaded {
	if (payload === undefined) return { kind: "ok", value: schema.defaults, migrated: false };
	if (version > schema.version) return { kind: "newer", version };
	return { kind: "ok", value: payload, migrated: false };
}

/** @internal The write seam: unit D3 replaces it with descriptor admission and freezing (rules 14, 15, 76). */
export function admitWrite(schema: Schema, value: unknown): object {
	if (!typeIs(value, "table")) error(`${schema.name}: a store's value is a record, got ${typeOf(value)}`, 0);
	return value;
}
