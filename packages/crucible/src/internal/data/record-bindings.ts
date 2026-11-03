import type { OperationId, StoreResult } from "../../data";
import type { UpdateResult } from "../../server/data";
import { bootClock } from "../boot";
import { CALL_DEADLINE_SECONDS } from "./limits";
import { dataPort } from "./port";
import type { DataEngine, StoreResult as EngineResult } from "./profiles";
import { declaredRecords, installRecordDriver } from "./record-declarations";
import { RecordStore, type UpdateOutcome } from "./records";
import { Waiters } from "./waiters";

const bound = new Map<string, RecordStore>();
let activation: Waiters | undefined;

/** @internal Rule 104: gives every declared record store its engine, and wakes calls waiting for activation. */
export function bindRecords(engine: DataEngine) {
	for (const [name, declaration] of declaredRecords()) bound.set(name, new RecordStore(engine, declaration.schema));
	activation?.wake();
}

/** @internal */
export function unbindRecords() {
	bound.clear();
}

/** Rule 4: before activation a call waits for it, within its own deadline. */
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

function publicUpdate(result: UpdateOutcome): UpdateResult<object> {
	return result.kind === "ok"
		? { ok: true, value: result.value, replayed: result.replayed }
		: { ok: false, reason: result.kind };
}

installRecordDriver({
	read(name: string, key: string) {
		const store = activated(name);
		if (store === undefined) return { ok: false, reason: "timeout" };
		return publicResult(store.read(key));
	},
	update(name: string, key: string, change: (current: object) => object, id?: OperationId) {
		const store = activated(name);
		if (store === undefined) return { ok: false, reason: "timeout" };
		return publicUpdate(store.update(key, change, id));
	},
});
