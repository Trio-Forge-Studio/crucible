import { startBatch } from "@rbxts/charm";
import { bootClock } from "../boot";
import { settle } from "../reactive/store";
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

/**
 * @internal A session's authority over its player's profile, which specs/profile models. A loading session may be
 * abandoned, and its load then ends it. A sealed session has stopped writing; its release starts at once, except while
 * a handoff or a release runs its onRelease claims. An ended session either never loaded or holds its release's outcome.
 */
export type Authority =
	| { readonly phase: Phase.Loading; readonly abandoned: boolean }
	| { readonly phase: Phase.Active }
	| { readonly phase: Phase.Sealed; readonly by: SealReason; readonly releasing: boolean }
	| { readonly phase: Phase.Ended; readonly by?: undefined; readonly released?: undefined }
	| {
			readonly phase: Phase.Ended;
			readonly by: SealReason;
			readonly released: Released;
			readonly ambiguous: boolean;
	  };

/** @internal */
export type In<P extends Phase> = Extract<Authority, { readonly phase: P }>;

/** @internal */
export const LOADING: In<Phase.Loading> = { phase: Phase.Loading, abandoned: false };
const ABANDONED: In<Phase.Loading> = { phase: Phase.Loading, abandoned: true };
const ACTIVE: In<Phase.Active> = { phase: Phase.Active };
const NEVER_LOADED: In<Phase.Ended> = { phase: Phase.Ended };

/** @internal The load ends its session once it sees this (rule 20). */
export function abandoned(_state: In<Phase.Loading>): In<Phase.Loading> {
	return ABANDONED;
}

/** @internal Rules 17 to 21: the acquired lease and onLoad's value make the session writable. */
export function activated(_state: In<Phase.Loading>): In<Phase.Active> {
	return ACTIVE;
}

/** @internal Rule 20: a load that stops or fails ends its session. */
export function loadEnded(_state: In<Phase.Loading>): In<Phase.Ended> {
	return NEVER_LOADED;
}

/** @internal Rule 26: a seal stops every write at once. */
export function sealed(_state: In<Phase.Active>, by: SealReason): In<Phase.Sealed> {
	return { phase: Phase.Sealed, by, releasing: false };
}

/** @internal The sealed session's release may be sent. */
export function releasing(state: In<Phase.Sealed>): In<Phase.Sealed> {
	return state.releasing ? state : { phase: Phase.Sealed, by: state.by, releasing: true };
}

/** @internal */
export type SettledRelease = Extract<In<Phase.Ended>, { readonly released: Released }>;

/** @internal Rule 29: the release settled, committed or not. */
export function releaseSettled(state: In<Phase.Sealed>, released: Released, ambiguous: boolean): SettledRelease {
	return { phase: Phase.Ended, by: state.by, released, ambiguous };
}

/** @internal Why the session was sealed, once it has been. */
export function sealedBy(state: Authority): SealReason | undefined {
	return state.phase === Phase.Sealed || state.phase === Phase.Ended ? state.by : undefined;
}

/** @internal How the release settled, once it has. */
export function releaseOutcome(state: Authority): Released | undefined {
	return state.phase === Phase.Ended ? state.released : undefined;
}

/** @internal An afterGrant effect (rule 61). */
export type Effect = () => void;

/** @internal */
export const enum Stage {
	Staged,
	Durable,
}

/**
 * @internal A purchase this session holds: staged in a revision no confirmed write holds yet, or durable in the last
 * envelope a confirming owned write returned or found (rule 28). The afterGrant effects its grant staged here wait
 * with it until the purchase is acknowledged (rule 61).
 */
export type Receipt =
	| { readonly stage: Stage.Staged; readonly effects: readonly Effect[] }
	| { readonly stage: Stage.Durable; readonly effects: readonly Effect[] };

const NO_EFFECTS: readonly Effect[] = [];
const DURABLE: Receipt = { stage: Stage.Durable, effects: NO_EFFECTS };

/** @internal Rule 59: a grant stages its id with its effects in one revision. */
export function receiptStaged(effects: readonly Effect[]): Receipt {
	return { stage: Stage.Staged, effects };
}

/** @internal Rule 28: a purchase is durable exactly while the confirmed envelope holds it. */
export function receiptConfirmed(receipt: Receipt | undefined, stored: boolean): Receipt {
	if (receipt === undefined) return DURABLE;
	if ((receipt.stage === Stage.Durable) === stored) return receipt;
	return { stage: stored ? Stage.Durable : Stage.Staged, effects: receipt.effects };
}

/** @internal Rule 61: acknowledging a durable purchase takes its effects, once. */
export function receiptAcknowledged(_receipt: Extract<Receipt, { readonly stage: Stage.Durable }>): Receipt {
	return DURABLE;
}

/** @internal The one owned write a session has in flight (rules 28, 31, 32). */
export interface Write {
	readonly ticket: Ticket;
	readonly release: boolean;
	/** The revision its latest attempt carries, which the scheduler may dispatch before `submit` returns. */
	readonly carrying: () => number;
}

const callbackThreads = new Set<thread>();
let depth = 0;

/**
 * @internal Runs a data callback on a no-yield worker whose thread may not write data (rule 13), inside a Charm batch
 * so no effect its writes reach runs until it has returned; a yield or throw is thrown here.
 */
export function dataCallback<A, B>(work: (a: A, b: B) => unknown, a: A, b: B): unknown {
	let worker: thread | undefined;
	depth += 1;
	startBatch();
	const [status, value] = call(
		(x: A, y: B) => {
			worker = coroutine.running();
			callbackThreads.add(worker);
			return work(x, y);
		},
		a,
		b,
	);
	depth -= 1;
	if (worker !== undefined) callbackThreads.delete(worker);
	settle("a data callback");
	if (status !== CallResult.Ok) error(value, 0);
	return value;
}

/**
 * @internal True while any data callback runs. Callbacks never yield, so code running then was started inside one,
 * on its thread or a thread it spawned (rule 123).
 */
export function insideDataCallback(): boolean {
	return depth > 0;
}

/** @internal Rule 13: no data write on a data callback's own thread. */
export function assertOutsideCallback() {
	if (callbackThreads.has(coroutine.running())) error("a write inside a data callback; return the data instead", 3);
}

/** @internal One player's profile on this server, and the only writer of its data (rules 12 to 16). */
export class Session {
	authority: Authority = LOADING;
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
	/** The stage of each purchase this session holds, and of any other the confirmed envelope holds. */
	readonly purchases = new Map<string, Receipt>();
	/** Applied inbox ids by the revision that applied them, kept until a confirmed write holds that revision (rule 72). */
	readonly acks = new Map<string, number>();
	/** At most one mark per item, against the generation last read (rule 72). */
	readonly marks = new Map<string, InboxMark>();
	/** The inbox of the newest envelope this session read, which acks and marks adjust (rules 71, 72). */
	inbox: readonly InboxItem[] = [];
	/** When an item may be attempted again, by id (rule 73). */
	readonly retryAt = new Map<string, number>();
	writing?: Write;
	/** The highest revision carried by an operation that gave up after an ambiguous attempt. */
	lost = NONE;
	want?: Priority;
	renewAt = 0;

	constructor(
		readonly schema: Schema,
		readonly player: Player,
		readonly userId: number,
		readonly token: string,
		private readonly hooks: { readonly changed: () => void; readonly publish: (value: object) => void },
	) {}

	writable(): boolean {
		return this.authority.phase === Phase.Active && bootClock() < this.safeUntil;
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
		const write = this.writing;
		const ticket = write?.ticket;
		const pending = ticket?.dispatched === true && (ticket.inFlight || ticket.ambiguous) ? write!.carrying() : NONE;
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
		this.schema.check?.(admitted, this.value);
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

	durable(id: string): boolean {
		return this.purchases.get(id)?.stage === Stage.Durable;
	}

	/** Rule 28: exactly the receipts of the envelope a confirming owned write returned or found are durable. */
	confirmReceipts(receipts: readonly string[]) {
		const stored = new Set(receipts);
		for (const [id, receipt] of this.purchases) this.purchases.set(id, receiptConfirmed(receipt, stored.has(id)));
		for (const id of receipts)
			if (!this.purchases.has(id)) this.purchases.set(id, receiptConfirmed(undefined, true));
	}
}
