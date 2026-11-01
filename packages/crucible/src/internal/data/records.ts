import { bootClock } from "../boot";
import { admitWrite, loadRecord, sealSchema, type Schema } from "./admission";
import { emptyEnvelope, envelopeBytes, jsonBytes, parseEnvelope, type Envelope } from "./envelope";
import {
	CALL_DEADLINE_SECONDS,
	KEY_MAX_BYTES,
	MAX_ENVELOPE_BYTES,
	OPERATION_ID_MAX_AGE_SECONDS,
	OPERATION_ID_MAX_BYTES,
	OPERATION_MARKERS_PER_KEY,
	RECORD_PAYLOAD_MAX_BYTES,
} from "./limits";
import { ageOf, createdAt, evictable, newOperationId } from "./operation-id";
import type { StoredValue } from "./port";
import type { DataEngine, StoreResult } from "./profiles";
import { Priority } from "./scheduler";
import { assertOutsideCallback, dataCallback } from "./session";

/** @internal The failure kinds of rule 90. */
export type StoreFailureKind = Exclude<StoreResult<never>["kind"], "ok">;

/** @internal A generic update's result: the stored record and whether it is a replay (rule 88). */
export type UpdateOutcome =
	| { readonly kind: "ok"; readonly value: object; readonly replayed: boolean }
	| { readonly kind: StoreFailureKind };

type UpdateDecision =
	| { readonly kind: "written"; readonly value: object; readonly bytes: number }
	| { readonly kind: "replay"; readonly envelope: Envelope }
	| { readonly kind: "expired" }
	| { readonly kind: "overloaded" }
	| { readonly kind: "newer"; readonly version: number }
	| { readonly kind: "unreadable"; readonly reason: string }
	| { readonly kind: "refused"; readonly error: unknown }
	| { readonly kind: "too-large"; readonly bytes: number };

interface UpdateInput {
	readonly schema: Schema;
	readonly key: string;
	readonly id: string;
	readonly change: (current: object) => object;
	readonly wall: () => number;
}

function stored(current: StoredValue | undefined, envelope: Envelope): StoredValue {
	return { value: envelope, userIds: current?.userIds ?? [], metadata: current?.metadata ?? {} };
}

/**
 * Rules 87 and 88 as one transform: it recognizes its own id, refuses an expired id or a young full ring, migrates and
 * admits the record, runs `change`, and stores the result with the id in one write.
 */
function updateRecord(
	current: StoredValue | undefined,
	input: UpdateInput,
	record: (decision: UpdateDecision) => void,
): StoredValue | undefined {
	const { schema, id } = input;
	const parsed = parseEnvelope(current?.value, "records");
	if (parsed.kind === "unreadable") {
		record(parsed);
		return undefined;
	}
	const envelope = parsed.kind === "ok" ? parsed.envelope : emptyEnvelope("records");
	if (envelope.payload !== undefined && envelope.schemaVersion > schema.version) {
		record({ kind: "newer", version: envelope.schemaVersion });
		return undefined;
	}
	if (envelope.operations.includes(id)) {
		record({ kind: "replay", envelope });
		return undefined;
	}
	const created = createdAt(id)!;
	if (created <= envelope.operationFloor) {
		record({ kind: "expired" });
		return undefined;
	}
	let operations = envelope.operations;
	let floor = envelope.operationFloor;
	if (operations.size() >= OPERATION_MARKERS_PER_KEY) {
		const oldest = operations[0];
		if (!evictable(oldest, input.wall())) {
			record({ kind: "overloaded" });
			return undefined;
		}
		operations = operations.filter((_, index) => index > 0);
		floor = math.max(floor, createdAt(oldest) ?? floor);
	}
	const loaded = loadRecord(schema, envelope.payload, envelope.schemaVersion);
	if (loaded.kind !== "ok") {
		record({ kind: "unreadable", reason: loaded.kind === "unreadable" ? loaded.reason : "newer" });
		return undefined;
	}
	const [ok, result] = pcall(() => {
		const changed = dataCallback(input.change, loaded.value, undefined);
		if (rawequal(changed, loaded.value)) return loaded.value;
		const admitted = admitWrite(schema, changed);
		const bytes = jsonBytes(admitted);
		if (bytes > RECORD_PAYLOAD_MAX_BYTES)
			error(`${schema.name}: ${input.key}'s record is ${bytes} bytes, over ${RECORD_PAYLOAD_MAX_BYTES}`, 0);
		return admitted;
	});
	if (!ok) {
		record({ kind: "refused", error: result });
		return undefined;
	}
	const value = result as object;
	const unchanged = rawequal(value, loaded.value);
	const keep = unchanged && !loaded.migrated;
	const written: Envelope = {
		...envelope,
		payload: keep ? envelope.payload : value,
		schemaVersion: keep ? envelope.schemaVersion : schema.version,
		operations: [...operations, id],
		operationFloor: floor,
	};
	const bytes = envelopeBytes(written);
	if (bytes > MAX_ENVELOPE_BYTES) {
		record({ kind: "too-large", bytes });
		return undefined;
	}
	record({ kind: "written", value, bytes });
	return stored(current, written);
}

/** @internal Keyed records with no session and no lease (rules 85 to 91): `read` and `update`, each one scheduled request. */
export class RecordStore {
	readonly schema: Schema;
	/** The last encoded size seen per key worth pacing, so a large record is paced by what it weighs (rule 36). */
	private readonly sizes = new Map<string, number>();

	constructor(
		private readonly engine: DataEngine,
		schema: Schema,
	) {
		this.schema = sealSchema(schema);
	}

	private check(key: unknown): string {
		if (!typeIs(key, "string") || key.size() < 1 || key.size() > KEY_MAX_BYTES)
			error(`${this.schema.name}: a key is 1 to ${KEY_MAX_BYTES} bytes`, 3);
		return key;
	}

	private weigh(key: string): number {
		return math.max(this.sizes.get(key) ?? 0, 1024);
	}

	private remember(key: string, bytes: number) {
		if (bytes > 4096) this.sizes.set(key, bytes);
		else this.sizes.delete(key);
	}

	/** Rule 110: once per store and code; the message names the store and key, never a value. */
	private unavailable(
		key: string,
		code: "unreadable" | "newer" | "large",
		reason: string,
	): { readonly kind: "unavailable" } {
		this.engine.diagnose(`record:${this.schema.name}:${code}`, `${this.schema.name}: ${key} ${reason}`);
		return { kind: "unavailable" };
	}

	/** Rule 86: one read at Ordinary priority, migrated and admitted, never written back. */
	read(key: string): StoreResult<object | undefined> {
		this.check(key);
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.schema.name,
			key,
			priority: Priority.Ordinary,
			kind: "read",
			dispatchDeadline: deadline,
			bytes: this.weigh(key),
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error") return { kind: "unavailable" };
		const parsed = parseEnvelope(settled.outcome.value?.value, "records");
		if (parsed.kind === "absent") return { kind: "ok", value: undefined };
		if (parsed.kind === "unreadable") return this.unavailable(key, "unreadable", "is unreadable");
		if (parsed.envelope.payload === undefined) return { kind: "ok", value: undefined };
		this.remember(key, envelopeBytes(parsed.envelope));
		const loaded = loadRecord(this.schema, parsed.envelope.payload, parsed.envelope.schemaVersion);
		if (loaded.kind === "newer") return this.unavailable(key, "newer", "was written by a newer build");
		if (loaded.kind === "unreadable") return this.unavailable(key, "unreadable", `did not load: ${loaded.reason}`);
		return { kind: "ok", value: loaded.value };
	}

	/** Rules 87 and 88. Yields; `change` may run several times. A throw in `change` is rethrown here. */
	update(key: string, change: (current: object) => object, id?: string): UpdateOutcome {
		assertOutsideCallback();
		this.check(key);
		const port = this.engine.port;
		const sent = id ?? newOperationId(port);
		if (!typeIs(sent, "string") || sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)
			error(
				`${this.schema.name}: an operation id comes from operationId() and is at most ${OPERATION_ID_MAX_BYTES} bytes`,
				2,
			);
		if (ageOf(sent, port.wall(), OPERATION_ID_MAX_AGE_SECONDS) !== "fresh") return { kind: "expired" };
		let decision: UpdateDecision | undefined;
		const input: UpdateInput = { schema: this.schema, key, id: sent, change, wall: port.wall };
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.schema.name,
			key,
			priority: Priority.Ordinary,
			kind: "update",
			dispatchDeadline: deadline,
			bytes: this.weigh(key),
			transform: () => (current) => updateRecord(current, input, (d) => (decision = d)),
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error" || decision === undefined) return { kind: "unavailable" };
		switch (decision.kind) {
			case "refused":
				return error(decision.error, 0);
			case "written":
				this.remember(key, decision.bytes);
				return { kind: "ok", value: decision.value, replayed: false };
			case "replay": {
				const loaded = loadRecord(this.schema, decision.envelope.payload, decision.envelope.schemaVersion);
				if (loaded.kind !== "ok") return { kind: "unavailable" };
				return { kind: "ok", value: loaded.value, replayed: true };
			}
			case "newer":
				return this.unavailable(key, "newer", "was written by a newer build");
			case "unreadable":
				return this.unavailable(key, "unreadable", `did not load: ${decision.reason}`);
			case "too-large":
				return this.unavailable(key, "large", `would make the envelope ${decision.bytes} bytes`);
			default:
				return { kind: decision.kind };
		}
	}
}
