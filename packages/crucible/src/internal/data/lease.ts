import { emptyEnvelope, envelopeBytes, parseEnvelope, type Envelope, type Lease } from "./envelope";
import { applyAcks, applyMarks, type InboxMark } from "./inbox-transforms";
import { LEASE_SECONDS, MAX_ENVELOPE_BYTES, TAKEOVER_GRACE_SECONDS } from "./limits";
import type { StoredValue } from "./port";

/** @internal What an owned write or a carrying acquisition stores for a session: its newest state (rules 27 to 29). */
export interface Snapshot {
	readonly revision: number;
	readonly schemaVersion: number;
	readonly payload: object;
	readonly receipts: readonly string[];
	/** Inbox ids applied in revisions up to `revision` (rule 72). */
	readonly acks: readonly string[];
	/** Staged attempt counts and quarantine moves, applied whatever the payload revision (rule 72). */
	readonly marks: readonly InboxMark[];
}

/** @internal A sealed session of this server whose release settled, keyed by its token in `supersedes` (rule 29). */
export interface Sealed {
	readonly epoch: number;
	readonly snapshot: Snapshot;
}

/** @internal */
export type AcquireDecision =
	| { readonly kind: "taken"; readonly envelope: Envelope; readonly carried: boolean }
	| { readonly kind: "already"; readonly envelope: Envelope }
	| { readonly kind: "held"; readonly lease: Lease }
	| { readonly kind: "newer"; readonly version: number }
	| { readonly kind: "unreadable"; readonly reason: string };

/** @internal */
export interface AcquireInput {
	readonly userId: number;
	readonly token: string;
	readonly serverId: string;
	readonly jobId: string;
	readonly schemaVersion: number;
	readonly supersedes: ReadonlyMap<string, Sealed>;
	readonly wall: () => number;
}

/** @internal */
export type OwnedDecision =
	| { readonly kind: "written"; readonly envelope: Envelope }
	| { readonly kind: "already"; readonly envelope: Envelope }
	| { readonly kind: "fenced"; readonly lease?: Lease }
	| { readonly kind: "expired" }
	| { readonly kind: "too-large"; readonly bytes: number }
	| { readonly kind: "unreadable"; readonly reason: string };

/** @internal */
export interface OwnedWrite {
	readonly userId: number;
	readonly token: string;
	readonly epoch: number;
	readonly op: number;
	readonly snapshot: Snapshot;
	readonly release: boolean;
	readonly wall: () => number;
}

function union(stored: readonly string[], staged: readonly string[]): readonly string[] {
	const seen = new Set(stored);
	const result = [...stored];
	for (const id of staged)
		if (!seen.has(id)) {
			seen.add(id);
			result.push(id);
		}
	return result;
}

/** @internal */
export function stored(current: StoredValue | undefined, userId: number, envelope: Envelope): StoredValue {
	const userIds = [...(current?.userIds ?? [])];
	if (!userIds.includes(userId)) userIds.push(userId);
	return { value: envelope, userIds, metadata: current?.metadata ?? {} };
}

function withSnapshot(envelope: Envelope, snapshot: Snapshot, commit: Envelope["commit"]): Envelope {
	return applyMarks(
		applyAcks(
			{
				...envelope,
				payload: snapshot.payload,
				schemaVersion: snapshot.schemaVersion,
				receipts: union(envelope.receipts, snapshot.receipts),
				commit,
			},
			snapshot.acks,
		),
		snapshot.marks,
	);
}

/** @internal Takes the lease only when absent, our own settled sealed token's, or past expiry plus grace (rule 24). */
export function acquire(
	current: StoredValue | undefined,
	input: AcquireInput,
	record: (decision: AcquireDecision) => void,
): StoredValue | undefined {
	const parsed = parseEnvelope(current?.value, "profiles");
	if (parsed.kind === "unreadable") {
		record(parsed);
		return undefined;
	}
	const envelope = parsed.kind === "ok" ? parsed.envelope : emptyEnvelope("profiles");
	if (envelope.payload !== undefined && envelope.schemaVersion > input.schemaVersion) {
		record({ kind: "newer", version: envelope.schemaVersion });
		return undefined;
	}
	const lease = envelope.lease;
	if (lease?.token === input.token) {
		record({ kind: "already", envelope });
		return undefined;
	}
	const sealed = lease === undefined ? undefined : input.supersedes.get(lease.token);
	const wall = input.wall();
	if (lease !== undefined && sealed === undefined && wall <= lease.expiresAt + TAKEOVER_GRACE_SECONDS) {
		record({ kind: "held", lease });
		return undefined;
	}
	let written: Envelope = {
		...envelope,
		epoch: envelope.epoch + 1,
		lease: { token: input.token, serverId: input.serverId, jobId: input.jobId, expiresAt: wall + LEASE_SECONDS },
	};
	const commit = envelope.commit;
	const carried =
		sealed !== undefined &&
		envelope.epoch === sealed.epoch &&
		(commit?.token !== lease!.token || commit.revision < sealed.snapshot.revision);
	if (carried)
		written = withSnapshot(written, sealed.snapshot, {
			token: lease!.token,
			revision: sealed.snapshot.revision,
			op: commit?.token === lease!.token ? commit.op : 0,
		});
	const bytes = envelopeBytes(written);
	if (bytes > MAX_ENVELOPE_BYTES) {
		record({ kind: "unreadable", reason: `envelope ${bytes} bytes` });
		return undefined;
	}
	record({ kind: "taken", envelope: written, carried });
	return stored(current, input.userId, written);
}

/** @internal A renewal, save, handoff or release: fenced on token and epoch, identified by `op` (rules 27, 28). */
export function ownedWrite(
	current: StoredValue | undefined,
	input: OwnedWrite,
	record: (decision: OwnedDecision) => void,
): StoredValue | undefined {
	const parsed = parseEnvelope(current?.value, "profiles");
	if (parsed.kind === "unreadable") {
		record(parsed);
		return undefined;
	}
	if (parsed.kind === "absent") {
		record({ kind: "fenced" });
		return undefined;
	}
	const envelope = parsed.envelope;
	const commit = envelope.commit;
	if (commit?.token === input.token && commit.op >= input.op) {
		record({ kind: "already", envelope });
		return undefined;
	}
	const lease = envelope.lease;
	if (
		lease === undefined ||
		lease.token !== input.token ||
		envelope.epoch !== input.epoch ||
		(envelope.payload !== undefined && envelope.schemaVersion > input.snapshot.schemaVersion)
	) {
		record({ kind: "fenced", lease });
		return undefined;
	}
	const wall = input.wall();
	if (wall >= lease.expiresAt) {
		record({ kind: "expired" });
		return undefined;
	}
	const revision = commit?.token === input.token ? commit.revision : -1;
	const marker = { token: input.token, revision: math.max(input.snapshot.revision, revision), op: input.op };
	const written: Envelope = {
		...(input.snapshot.revision > revision
			? withSnapshot(envelope, input.snapshot, marker)
			: applyMarks(envelope, input.snapshot.marks)),
		lease: input.release ? undefined : { ...lease, expiresAt: wall + LEASE_SECONDS },
		commit: marker,
	};
	const bytes = envelopeBytes(written);
	if (bytes > MAX_ENVELOPE_BYTES) {
		record({ kind: "too-large", bytes });
		return undefined;
	}
	record({ kind: "written", envelope: written });
	return stored(current, input.userId, written);
}

/** @internal Removes exactly the abandoned load's own lease and writes nothing else (rule 20). */
export function abandonAcquisition(
	current: StoredValue | undefined,
	userId: number,
	token: string,
	record: (decision: "released" | "absent" | "foreign") => void,
): StoredValue | undefined {
	const parsed = parseEnvelope(current?.value, "profiles");
	if (parsed.kind !== "ok") {
		record("absent");
		return undefined;
	}
	if (parsed.envelope.lease?.token !== token) {
		record("foreign");
		return undefined;
	}
	record("released");
	return stored(current, userId, { ...parsed.envelope, lease: undefined });
}
