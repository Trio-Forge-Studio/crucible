import {
	INBOX_MAX_MESSAGES,
	MAX_PURCHASE_IDS,
	OPERATION_ID_MAX_BYTES,
	OPERATION_MARKERS_PER_KEY,
	PURCHASE_ID_MAX_BYTES,
	QUARANTINE_MAX_MESSAGES,
	SEEN_MAX_MESSAGES,
} from "./limits";

/** @internal */
export interface Lease {
	readonly token: string;
	readonly serverId: string;
	readonly jobId: string;
	readonly expiresAt: number;
}

/** @internal The last owned operation that committed (rule 28). */
export interface CommitMarker {
	readonly token: string;
	readonly revision: number;
	readonly op: number;
}

/** @internal */
export interface InboxItem {
	readonly id: string;
	readonly name: string;
	readonly version: number;
	readonly payload: unknown;
	readonly generation: number;
	readonly attempts: number;
}

/** @internal */
export interface QuarantinedItem extends InboxItem {
	readonly reason: "payload" | "attempts";
}

/** @internal */
export type EnvelopeKind = "profiles" | "records";

/** @internal One JSON value per key (rule 38). `payload` is the stored record exactly; bookkeeping is never inside it. */
export interface Envelope {
	readonly format: 1;
	readonly kind: EnvelopeKind;
	readonly schemaVersion: number;
	readonly payload?: object;
	readonly epoch: number;
	readonly lease?: Lease;
	readonly commit?: CommitMarker;
	readonly receipts: readonly string[];
	readonly inbox: readonly InboxItem[];
	readonly quarantine: readonly QuarantinedItem[];
	readonly seen: readonly string[];
	readonly seenFloor: number;
	readonly operations: readonly string[];
	readonly operationFloor: number;
}

/** @internal */
export type Parsed =
	| { readonly kind: "absent" }
	| { readonly kind: "ok"; readonly envelope: Envelope }
	| { readonly kind: "unreadable"; readonly reason: string };

const UNREADABLE = {};

function fail(reason: string): never {
	throw [UNREADABLE, reason];
}

function counter(value: unknown, name: string): number {
	if (!typeIs(value, "number") || value < 0 || value % 1 !== 0) fail(`bad ${name}`);
	return value;
}

function time(value: unknown, name: string): number {
	if (!typeIs(value, "number") || value !== value || value < 0) fail(`bad ${name}`);
	return value;
}

function text(value: unknown, name: string, max = math.huge): string {
	if (!typeIs(value, "string") || value.size() === 0 || value.size() > max) fail(`bad ${name}`);
	return value;
}

/** Counts entries before reading any, stopping at the first one past `cap`. */
function list(value: unknown, name: string, cap: number): readonly defined[] {
	if (value === undefined) return [];
	if (!typeIs(value, "table")) fail(`bad ${name}`);
	let count = 0;
	for (const [key] of pairs(value)) {
		count += 1;
		if (count > cap) fail(`${name} over ${cap}`);
		if (!typeIs(key, "number") || key % 1 !== 0 || key < 1) fail(`bad ${name}`);
	}
	const items = value as defined[];
	if (items.size() !== count) fail(`bad ${name}`);
	return items;
}

function ids(value: unknown, name: string, cap: number, max: number): readonly string[] {
	return list(value, name, cap).map((id) => text(id, `${name} id`, max));
}

function item(raw: unknown, name: string): InboxItem {
	if (!typeIs(raw, "table")) fail(`bad ${name} item`);
	const t = raw as Record<string, unknown>;
	return {
		id: text(t.id, `${name} id`, OPERATION_ID_MAX_BYTES),
		name: text(t.name, `${name} name`),
		version: counter(t.version, `${name} version`),
		payload: t.payload,
		generation: counter(t.generation, `${name} generation`),
		attempts: counter(t.attempts, `${name} attempts`),
	};
}

function readEnvelope(raw: unknown, kind: EnvelopeKind): Envelope {
	if (!typeIs(raw, "table")) fail("not a table");
	const t = raw as Record<string, unknown>;
	if (t.format !== 1) fail(`format ${tostring(t.format)}`);
	if (t.kind !== kind) fail(`kind ${tostring(t.kind)}`);
	const payload = t.payload;
	if (payload !== undefined && !typeIs(payload, "table")) fail("bad payload");
	let lease: Lease | undefined;
	if (t.lease !== undefined) {
		if (!typeIs(t.lease, "table")) fail("bad lease");
		const l = t.lease as Record<string, unknown>;
		lease = {
			token: text(l.token, "lease token"),
			serverId: text(l.serverId, "lease server"),
			jobId: typeIs(l.jobId, "string") ? l.jobId : fail("bad lease job"),
			expiresAt: time(l.expiresAt, "lease expiry"),
		};
	}
	let commit: CommitMarker | undefined;
	if (t.commit !== undefined) {
		if (!typeIs(t.commit, "table")) fail("bad commit");
		const c = t.commit as Record<string, unknown>;
		commit = {
			token: text(c.token, "commit token"),
			revision: counter(c.revision, "commit revision"),
			op: counter(c.op, "commit op"),
		};
	}
	return {
		format: 1,
		kind,
		schemaVersion: counter(t.schemaVersion, "schemaVersion"),
		payload: payload as object | undefined,
		epoch: counter(t.epoch, "epoch"),
		lease,
		commit,
		receipts: ids(t.receipts, "receipts", MAX_PURCHASE_IDS, PURCHASE_ID_MAX_BYTES),
		inbox: list(t.inbox, "inbox", INBOX_MAX_MESSAGES).map((raw) => item(raw, "inbox")),
		quarantine: list(t.quarantine, "quarantine", QUARANTINE_MAX_MESSAGES).map((raw) => {
			const reason = (raw as { reason?: unknown }).reason;
			if (reason !== "payload" && reason !== "attempts") fail("bad quarantine reason");
			return { ...item(raw, "quarantine"), reason };
		}),
		seen: ids(t.seen, "seen", SEEN_MAX_MESSAGES, OPERATION_ID_MAX_BYTES),
		seenFloor: time(t.seenFloor ?? 0, "seenFloor"),
		operations: ids(t.operations, "operations", OPERATION_MARKERS_PER_KEY, OPERATION_ID_MAX_BYTES),
		operationFloor: time(t.operationFloor ?? 0, "operationFloor"),
	};
}

/** @internal Validates before anything is interpreted; a malformed envelope is never repaired (rule 40). */
export function parseEnvelope(raw: unknown, kind: EnvelopeKind): Parsed {
	if (raw === undefined) return { kind: "absent" };
	const [ok, result] = pcall(readEnvelope, raw, kind);
	if (ok) return { kind: "ok", envelope: result };
	const [marker, reason] = result as [unknown, string];
	if (marker === UNREADABLE) return { kind: "unreadable", reason };
	throw result;
}

/** @internal An absent key: unopened, epoch 0, empty collections. */
export function emptyEnvelope(kind: EnvelopeKind): Envelope {
	return {
		format: 1,
		kind,
		schemaVersion: 0,
		epoch: 0,
		receipts: [],
		inbox: [],
		quarantine: [],
		seen: [],
		seenFloor: 0,
		operations: [],
		operationFloor: 0,
	};
}

const SHORT_ESCAPES = new Set(['"', "\\", "\b", "\f", "\n", "\r", "\t"]);

function stringBytes(value: string): number {
	let bytes = value.size() + 2;
	for (const [special] of value.gmatch('[%c"\\]')) bytes += SHORT_ESCAPES.has(special as string) ? 1 : 5;
	return bytes;
}

/** @internal Encoded JSON bytes, an upper bound on what Roblox counts (rule 39). */
export function jsonBytes(value: unknown): number {
	if (typeIs(value, "string")) return stringBytes(value);
	if (typeIs(value, "number")) return (value % 1 === 0 ? tostring(value) : "%.17g".format(value)).size();
	if (typeIs(value, "boolean")) return value ? 4 : 5;
	if (!typeIs(value, "table")) return 4;
	const t = value as Record<string | number, unknown>;
	const size = (t as unknown as unknown[]).size();
	let entries = 0;
	let bytes = 2;
	for (const [key, entry] of pairs(t)) {
		entries += 1;
		bytes += jsonBytes(entry);
		if (size === 0 || !typeIs(key, "number")) bytes += stringBytes(tostring(key)) + 1;
	}
	return bytes + math.max(entries - 1, 0);
}

/** @internal */
export function envelopeBytes(envelope: Envelope): number {
	return jsonBytes(envelope);
}
