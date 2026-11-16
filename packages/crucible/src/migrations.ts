import type { Json } from "./data";

declare const deleteKind: unique symbol;
declare const replacementKind: unique symbol;

export interface MigrationDelete {
	readonly [deleteKind]: true;
}

export interface MigrationReplacement<T extends Json = Json> {
	readonly [replacementKind]: T;
}

export interface MigrationPatch {
	readonly [key: string]: Json | MigrationPatch | MigrationDelete | MigrationReplacement | undefined;
}

// A locked metatable keeps admission refusing an operation outside a patch and leaves nothing to forge one from.
const LOCKED = { __metatable: "a migration operation" };
const replacements = new WeakMap<object, Json>();

export const migrationDelete = table.freeze(setmetatable({}, table.freeze({}))) as MigrationDelete;

export function migrationReplace<T extends Json>(value: T): MigrationReplacement<T> {
	if (value === undefined) error("migrationReplace requires a JSON value, got nil", 0);
	const replacement = setmetatable({}, LOCKED);
	replacements.set(replacement, value);
	return replacement as unknown as MigrationReplacement<T>;
}

/** @internal */
export function isMigrationDelete(value: unknown): boolean {
	return value === migrationDelete;
}

/** @internal */
export function isMigrationReplacement(value: unknown): boolean {
	return typeIs(value, "table") && replacements.has(value);
}

/** @internal The value `migrationReplace` was given. */
export function replacementValue(replacement: object): Json | undefined {
	return replacements.get(replacement);
}
