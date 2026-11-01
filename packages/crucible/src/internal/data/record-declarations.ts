import type { Migration, OperationId, StoreResult } from "../../data";
import type { DataStore, UpdateResult } from "../../server/data";
import type { Node } from "../network/schema";
import { sealSchema, type MigrationStep, type Schema } from "./admission";
import { MAX_DATA_DECLARATIONS, STORE_NAME_MAX_BYTES } from "./limits";

/** @internal */
export interface RecordDeclaration {
	readonly name: string;
	readonly schema: Schema;
}

/**
 * @internal What a declared store's calls run on once DataRuntime activates. This module is reached from the emit
 * entry, so it imports nothing that loads boot; `record-bindings.ts` installs the driver.
 */
export interface RecordDriver {
	read(name: string, key: string): StoreResult<object | undefined>;
	update(name: string, key: string, change: (current: object) => object, id?: OperationId): UpdateResult<object>;
}

const declarations = new Map<string, RecordDeclaration>();
let sealed = false;
let driver: RecordDriver | undefined;

/** Rule 2's grammar: dot-separated `[A-Za-z][A-Za-z0-9_]*` segments, with `Crucible` reserved as the first. */
function checkName(name: string) {
	if (name.size() === 0 || name.size() > STORE_NAME_MAX_BYTES)
		error(`${name}: a store name is 1 to ${STORE_NAME_MAX_BYTES} bytes`, 4);
	const segments = name.split(".");
	for (const segment of segments)
		if (segment.match("^[A-Za-z][A-Za-z0-9_]*$")[0] === undefined)
			error(`${name}: a store name is dot-separated segments of letters, digits and underscores`, 4);
	if (segments[0] === "Crucible") error(`${name}: the name Crucible is reserved`, 4);
}

/** @internal Rule 3: DataRuntime seals the registry when it constructs; a failed boot unseals it. */
export function sealDeclarations(value: boolean) {
	sealed = value;
}

/** @internal The declarations a DataRuntime binds at activation (rule 104). */
export function declaredRecords(): ReadonlyMap<string, RecordDeclaration> {
	return declarations;
}

/** @internal */
export function installRecordDriver(value: RecordDriver | undefined) {
	driver = value;
}

function installed(name: string): RecordDriver {
	if (driver === undefined) error(`${name}: the data runtime is not loaded; import @trioforge/crucible/server`, 3);
	return driver;
}

/**
 * @internal Rules 4 and 85: the emit form's body. Admits and freezes `defaults` with `value`, registers the name, and
 * returns a frozen handle whose two calls bind to the engine at activation.
 */
export function declareRecords<T extends object>(
	name: string,
	defaults: T,
	migrations: readonly Migration[] | undefined,
	value: Node,
): DataStore<T> {
	checkName(name);
	if (sealed)
		error(`${name} was declared after the data registry sealed; declare stores in modules that boot loads`, 3);
	if (declarations.has(name)) error(`${name} is already declared`, 3);
	if (declarations.size() >= MAX_DATA_DECLARATIONS)
		error(`${name}: more than ${MAX_DATA_DECLARATIONS} data declarations`, 3);
	const steps = [...(migrations ?? [])] as unknown as MigrationStep[];
	const schema = sealSchema({ name, version: steps.size(), defaults, node: value, migrations: steps });
	declarations.set(name, { name, schema });
	return table.freeze({
		read(key: string) {
			return installed(name).read(name, key) as StoreResult<T | undefined>;
		},
		update(key: string, change: (current: T) => T, id?: OperationId) {
			return installed(name).update(
				name,
				key,
				change as unknown as (current: object) => object,
				id,
			) as UpdateResult<T>;
		},
	}) as unknown as DataStore<T>;
}
