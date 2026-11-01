import { bootClock } from "../boot";
import type { DataStore, UpdateResult } from "../../server/data";
import type { Migration, OperationId, StoreResult } from "../../data";
import type { Node } from "../network/schema";
import { sealSchema, type MigrationStep } from "./admission";
import { CALL_DEADLINE_SECONDS, MAX_DATA_DECLARATIONS, STORE_NAME_MAX_BYTES } from "./limits";
import { dataPort } from "./port";
import type { DataEngine, StoreResult as EngineResult } from "./profiles";
import { RecordStore, type UpdateOutcome } from "./records";
import { Waiters } from "./waiters";

interface Declaration {
	readonly name: string;
	readonly schema: ReturnType<typeof sealSchema>;
}

const declarations = new Map<string, Declaration>();
const bound = new Map<string, RecordStore>();
let sealed = false;
let activation: Waiters | undefined;

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
export function declaredRecords(): ReadonlyMap<string, Declaration> {
	return declarations;
}

/** @internal Rule 104: gives every declared record store its engine, and wakes calls waiting for activation. */
export function bindRecords(engine: DataEngine) {
	for (const [name, declaration] of declarations) bound.set(name, new RecordStore(engine, declaration.schema));
	activation?.wake();
}

/** @internal */
export function unbindRecords() {
	bound.clear();
}

function activated(name: string): RecordStore | undefined {
	const deadline = bootClock() + CALL_DEADLINE_SECONDS;
	while (bound.get(name) === undefined && bootClock() < deadline) {
		activation ??= new Waiters(dataPort());
		activation.park(deadline);
	}
	return bound.get(name);
}

function publicResult<T>(result: EngineResult<T>): StoreResult<T> {
	return result.kind === "ok" ? { ok: true, value: result.value } : { ok: false, reason: result.kind };
}

function publicUpdate<T>(result: UpdateOutcome): UpdateResult<T> {
	return result.kind === "ok"
		? { ok: true, value: result.value as T, replayed: result.replayed }
		: { ok: false, reason: result.kind };
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
			const store = activated(name);
			if (store === undefined) return { ok: false, reason: "timeout" } as const;
			return publicResult(store.read(key)) as StoreResult<T | undefined>;
		},
		update(key: string, change: (current: T) => T, id?: OperationId) {
			const store = activated(name);
			if (store === undefined) return { ok: false, reason: "timeout" } as const;
			return publicUpdate<T>(store.update(key, change as unknown as (current: object) => object, id));
		},
	}) as unknown as DataStore<T>;
}
