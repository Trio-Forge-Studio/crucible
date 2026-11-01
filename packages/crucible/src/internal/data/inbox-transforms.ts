import { emptyEnvelope, envelopeBytes, jsonBytes, parseEnvelope, type Envelope, type InboxItem } from "./envelope";
import {
	INBOX_MAX_BYTES,
	INBOX_MAX_MESSAGES,
	MAX_ENVELOPE_BYTES,
	MESSAGE_ID_MAX_AGE_SECONDS,
	QUARANTINE_MAX_BYTES,
	QUARANTINE_MAX_MESSAGES,
	SEEN_MAX_MESSAGES,
} from "./limits";
import { ageOf, createdAt, evictable } from "./operation-id";
import type { StoredValue } from "./port";

/**
 * @internal An attempt count or a quarantine move for one item, compare-and-set on the item's generation: it applies
 * only while the stored item is live with exactly `generation` (rule 72).
 */
export interface InboxMark {
	readonly id: string;
	readonly generation: number;
	readonly attempts: number;
	readonly quarantine?: "payload" | "attempts";
}

function addSeen(envelope: Envelope, ids: readonly string[]): Pick<Envelope, "seen" | "seenFloor"> {
	const seen = [...envelope.seen];
	let floor = envelope.seenFloor;
	for (const id of ids) if (!seen.includes(id)) seen.push(id);
	while (seen.size() > SEEN_MAX_MESSAGES) {
		const evicted = seen.shift()!;
		floor = math.max(floor, createdAt(evicted) ?? 0);
	}
	return { seen, seenFloor: floor };
}

/** @internal Each id leaves the inbox or the quarantine, wherever it is, and enters seen as its exact string (rule 72). */
export function applyAcks(envelope: Envelope, acks: readonly string[]): Envelope {
	const gone = new Set(acks);
	return {
		...envelope,
		inbox: envelope.inbox.filter((item) => !gone.has(item.id)),
		quarantine: envelope.quarantine.filter((item) => !gone.has(item.id)),
		...addSeen(envelope, acks),
	};
}

/**
 * @internal Rule 72's compare-and-set. A move into a full quarantine leaves the item live with its attempts updated.
 * A mark whose generation no longer matches skips, and the item is untouched.
 */
export function applyMarks(envelope: Envelope, marks: readonly InboxMark[]): Envelope {
	let inbox = envelope.inbox;
	let quarantine = envelope.quarantine;
	for (const mark of marks) {
		const index = inbox.findIndex((item) => item.id === mark.id && item.generation === mark.generation);
		if (index < 0) continue;
		const item = inbox[index];
		const marked = { ...item, attempts: mark.attempts, generation: item.generation + 1 };
		const moved = [...quarantine, { ...marked, reason: mark.quarantine! }];
		const room =
			mark.quarantine !== undefined &&
			moved.size() <= QUARANTINE_MAX_MESSAGES &&
			jsonBytes(moved) <= QUARANTINE_MAX_BYTES;
		const rest = [...inbox];
		if (room) {
			rest.remove(index);
			quarantine = moved;
		} else rest[index] = marked;
		inbox = rest;
	}
	return inbox === envelope.inbox ? envelope : { ...envelope, inbox, quarantine };
}

/** @internal Exact structural equality of two stored JSON values (rule 68). */
export function sameValue(a: unknown, b: unknown): boolean {
	if (!typeIs(a, "table") || !typeIs(b, "table")) return a === b;
	let count = 0;
	for (const [key, value] of pairs(a as Record<string, unknown>)) {
		count += 1;
		if (!sameValue(value, (b as Record<string, unknown>)[key as string])) return false;
	}
	for (const _ of pairs(b as Record<string, unknown>)) count -= 1;
	return count === 0;
}

function stored(current: StoredValue | undefined, userId: number, envelope: Envelope): StoredValue {
	const userIds = [...(current?.userIds ?? [])];
	if (!userIds.includes(userId)) userIds.push(userId);
	return { value: envelope, userIds, metadata: current?.metadata ?? {} };
}

/** @internal */
export type AppendDecision =
	| { readonly kind: "appended" | "duplicate" | "reused" | "expired" | "overloaded" | "full" }
	| { readonly kind: "unreadable"; readonly reason: string };

/**
 * @internal An inbox append (rules 66 to 70). Touches only `inbox`, needs no lease and works on any stored version. An
 * absent key becomes an unopened envelope.
 */
export function append(
	current: StoredValue | undefined,
	userId: number,
	item: InboxItem,
	wall: () => number,
	record: (decision: AppendDecision) => void,
): StoredValue | undefined {
	const parsed = parseEnvelope(current?.value, "profiles");
	if (parsed.kind === "unreadable") {
		record(parsed);
		return undefined;
	}
	const envelope = parsed.kind === "ok" ? parsed.envelope : emptyEnvelope("profiles");
	const held =
		envelope.inbox.find((other) => other.id === item.id) ??
		envelope.quarantine.find((other) => other.id === item.id);
	if (held !== undefined) {
		record({ kind: held.name === item.name && sameValue(held.payload, item.payload) ? "duplicate" : "reused" });
		return undefined;
	}
	if (envelope.seen.includes(item.id)) {
		record({ kind: "duplicate" });
		return undefined;
	}
	const now = wall();
	if (ageOf(item.id, now, MESSAGE_ID_MAX_AGE_SECONDS) !== "fresh" || createdAt(item.id)! <= envelope.seenFloor) {
		record({ kind: "expired" });
		return undefined;
	}
	const inbox = [...envelope.inbox, item];
	if (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {
		record({ kind: "full" });
		return undefined;
	}
	if (envelope.seen.size() >= SEEN_MAX_MESSAGES && !evictable(envelope.seen[0], now)) {
		record({ kind: "overloaded" });
		return undefined;
	}
	const written = { ...envelope, inbox };
	if (envelopeBytes(written) > MAX_ENVELOPE_BYTES) {
		record({ kind: "full" });
		return undefined;
	}
	record({ kind: "appended" });
	return stored(current, userId, written);
}

/** @internal */
export type ClearDecision =
	| { readonly kind: "cleared"; readonly changed: number }
	| { readonly kind: "full" }
	| { readonly kind: "unreadable"; readonly reason: string };

/** @internal clearQuarantined (rule 73): touches only `inbox`, `quarantine` and `seen`, with no lease. */
export function clearQuarantine(
	current: StoredValue | undefined,
	userId: number,
	ids: ReadonlySet<string>,
	action: "retry" | "discard",
	record: (decision: ClearDecision) => void,
): StoredValue | undefined {
	const parsed = parseEnvelope(current?.value, "profiles");
	if (parsed.kind === "unreadable") {
		record(parsed);
		return undefined;
	}
	if (parsed.kind === "absent") {
		record({ kind: "cleared", changed: 0 });
		return undefined;
	}
	const envelope = parsed.envelope;
	const listed = envelope.quarantine.filter((item) => ids.has(item.id));
	if (listed.size() === 0) {
		record({ kind: "cleared", changed: 0 });
		return undefined;
	}
	const quarantine = envelope.quarantine.filter((item) => !ids.has(item.id));
	let written: Envelope;
	if (action === "retry") {
		const inbox = [
			...envelope.inbox,
			...listed.map((item) => ({
				id: item.id,
				name: item.name,
				version: item.version,
				payload: item.payload,
				generation: item.generation + 1,
				attempts: 0,
			})),
		];
		if (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {
			record({ kind: "full" });
			return undefined;
		}
		written = { ...envelope, inbox, quarantine };
	} else
		written = {
			...envelope,
			quarantine,
			...addSeen(
				envelope,
				listed.map((item) => item.id),
			),
		};
	record({ kind: "cleared", changed: listed.size() });
	return stored(current, userId, written);
}
