import { bootClock } from "../boot";
import { call, CallResult } from "../runner";
import { admitWrite, type Schema } from "./admission";
import { jsonBytes, type InboxItem } from "./envelope";
import type { InboxMark } from "./inbox-transforms";
import type { Snapshot } from "./lease";
import { PAYLOAD_MAX_BYTES } from "./limits";
import type { Priority, Ticket } from "./scheduler";

/** @internal `confirmed` before any owned write with the session's token is confirmed (rule 37). */
export const NONE = -1;

/**
 * @internal
 *   Loading --acquired, onLoad--> Active --leave / handoff / final phase / authority lost--> Sealed --release--> Ended
 *   Loading --failed, left or closed: abandon only--> Ended
 */
export const enum Phase {
	Loading,
	Active,
	Sealed,
	Ended,
}

/** @internal */
export type SealReason = "leave" | "handoff" | "shutdown" | "authority" | "fenced" | "retired";

/** @internal How a release settled: its write committed, or it was fenced or gave up (rule 29). */
export type Released = "committed" | "fenced" | "given up";

const callbackThreads = new Set<thread>();

/** @internal Runs a data callback on a no-yield worker whose thread may not write data (rule 13); a yield or throw is thrown here. */
export function dataCallback<A, B>(work: (a: A, b: B) => unknown, a: A, b: B): unknown {
	let worker: thread | undefined;
	const [status, value] = call(
		(x: A, y: B) => {
			worker = coroutine.running();
			callbackThreads.add(worker);
			return work(x, y);
		},
		a,
		b,
	);
	if (worker !== undefined) callbackThreads.delete(worker);
	if (status !== CallResult.Ok) error(value, 0);
	return value;
}

/** @internal Rule 13: no data write on a data callback's own thread. */
export function assertOutsideCallback() {
	if (callbackThreads.has(coroutine.running())) error("a write inside a data callback; return the data instead", 3);
}

/** @internal One player's profile on this server, and the only writer of its data (rules 12 to 16). */
export class Session {
	phase = Phase.Loading;
	sealedBy?: SealReason;
	abandoned = false;
	value!: object;
	epoch = 0;
	safeUntil = 0;
	/** Per token: 0 is the value as loaded, never carried from a previous session (rule 37). */
	revision = 0;
	confirmed = NONE;
	op = 0;
	/** From a data callback's start until its result is committed or refused (rule 13). */
	busy = false;
	/** Every purchase id this session holds, staged or durable, in staging order. */
	readonly receipts = new Array<string>();
	/** Exactly the receipts of the last envelope a confirming owned write returned or found (rule 28). */
	durable = new Set<string>();
	/** afterGrant effects staged with their grant, run once its id is durable (rule 61). */
	readonly effects = new Map<string, Array<() => void>>();
	/** Applied inbox ids by the revision that applied them, kept until a confirmed write holds that revision (rule 72). */
	readonly acks = new Map<string, number>();
	/** At most one mark per item, against the generation last read (rule 72). */
	readonly marks = new Map<string, InboxMark>();
	/** The inbox of the newest envelope this session read, which acks and marks adjust (rules 71, 72). */
	inbox: readonly InboxItem[] = [];
	/** When an item may be attempted again, by id (rule 73). */
	readonly retryAt = new Map<string, number>();
	writing?: Ticket;
	/** The revision the current operation's latest attempt carries. */
	carrying = NONE;
	/** The highest revision carried by an operation that gave up after an ambiguous attempt. */
	lost = NONE;
	want?: Priority;
	releaseWanted = false;
	released?: Released;
	releaseAmbiguous = false;
	renewAt = 0;

	constructor(
		readonly schema: Schema,
		readonly player: Player,
		readonly userId: number,
		readonly token: string,
		private readonly hooks: { readonly changed: () => void; readonly publish: (value: object) => void },
	) {}

	writable(): boolean {
		return this.phase === Phase.Active && bootClock() < this.safeUntil;
	}

	snapshot(): Snapshot {
		const applied = new Array<[string, number]>();
		for (const [id, revision] of this.acks) applied.push([id, revision]);
		applied.sort((a, b) => a[1] < b[1]);
		const acks = applied.map(([id]) => id);
		const marks = new Array<InboxMark>();
		for (const [, mark] of this.marks) marks.push(mark);
		return {
			revision: this.revision,
			schemaVersion: this.schema.version,
			payload: this.value,
			receipts: [...this.receipts],
			acks,
			marks,
		};
	}

	/** The highest revision an unresolved attempt may have stored: rule 37's "uncertain". */
	uncertain(): number {
		const ticket = this.writing;
		const pending = ticket?.dispatched === true && (ticket.inFlight || ticket.ambiguous) ? this.carrying : NONE;
		return math.max(pending, this.lost);
	}

	assertIdle() {
		if (this.busy)
			error(
				`${this.schema.name}: ${this.userId}'s profile is inside a data callback; return the data instead`,
				3,
			);
	}

	/** Runs `work` as a data callback: no yield, no data write on its thread, and the session busy throughout. */
	transact<R>(work: () => R): R {
		this.assertIdle();
		this.busy = true;
		const [ok, result] = pcall(work);
		this.busy = false;
		this.hooks.changed();
		if (!ok) error(result, 0);
		return result;
	}

	callback<A, B>(work: (a: A, b: B) => unknown, a: A, b: B): unknown {
		return dataCallback(work, a, b);
	}

	/** Rule 14's prepare: throws naming the store, never the value, and changes nothing. */
	prepare(value: unknown): object {
		const admitted = admitWrite(this.schema, value);
		const bytes = jsonBytes(admitted);
		if (bytes > PAYLOAD_MAX_BYTES)
			error(`${this.schema.name}: ${this.userId}'s profile is ${bytes} bytes, over ${PAYLOAD_MAX_BYTES}`, 0);
		return admitted;
	}

	/** Rule 14's commit: cannot throw and runs no user code. */
	commit(value: object, stage?: () => void) {
		this.value = value;
		stage?.();
		this.revision += 1;
	}

	/** Rules 12 to 14 at the engine level, without the writable check, which the runtime owns. */
	write(change: (current: object) => object, stage?: () => void): boolean {
		return this.transact(() => {
			const current = this.value;
			const result = this.callback(change, current, undefined);
			if (rawequal(result, current) && stage === undefined) return false;
			this.commit(this.prepare(result), stage);
			this.hooks.publish(this.value);
			return true;
		});
	}

	stage(id: string) {
		if (!this.receipts.includes(id)) this.receipts.push(id);
	}
}
