import { bootClock } from "../boot";
import {
	BUDGET_RECHECK_SECONDS,
	BUDGET_RESERVE_FRACTION,
	BUDGET_RESERVE_MIN,
	KEY_READ_BYTES_PER_MINUTE,
	KEY_WRITE_BYTES_PER_MINUTE,
	LEASE_CRITICAL_SECONDS,
	MAX_IN_FLIGHT,
	MAX_PENDING_OPERATIONS,
	MAX_PENDING_PER_KEY,
	ORDINARY_AGING_SECONDS,
	ORDINARY_MAX_IN_FLIGHT,
	RESERVED_SESSION_SLOTS,
	RETRY_BASE_SECONDS,
	RETRY_CAP_SECONDS,
} from "./limits";
import type { DataPort, StorageOutcome, StoredValue, Transform } from "./port";
import { Waiters } from "./waiters";

/** @internal Dispatch order, most urgent first (rule 34). */
export const enum Priority {
	LeaseCritical,
	Release,
	Receipt,
	Renew,
	Load,
	Ordinary,
}

/** @internal */
export interface Request {
	readonly store: string;
	readonly key: string;
	readonly priority: Priority;
	readonly kind: "update" | "read";
	/** No dispatch at or after it. Session work passes its safe bound (rule 36). */
	readonly dispatchDeadline: number;
	/** Orders Release and Renew, and makes a renewal LeaseCritical within 30 s of it (rule 34). */
	readonly safeUntil?: number;
	readonly renewal?: boolean;
	readonly bytes: number;
	/** Rebuilt for each attempt, so a retry carries the newest snapshot of the same operation (rule 28). */
	readonly transform?: () => Transform;
	readonly onDispatch?: (at: number) => void;
	readonly onResult?: (outcome: StorageOutcome<StoredValue | undefined>) => void;
}

/** @internal */
export type Settled =
	| { readonly kind: "done"; readonly outcome: StorageOutcome<StoredValue | undefined> }
	| { readonly kind: "timeout" }
	| { readonly kind: "uncertain" }
	| { readonly kind: "overloaded" }
	| { readonly kind: "closing" };

/** @internal */
export class Ticket {
	settled?: Settled;
	dispatched = false;
	inFlight = false;
	/** An attempt failed in a way that may have committed. */
	ambiguous = false;
	private readonly listeners = new Array<(settled: Settled) => void>();
	private readonly waiters: Waiters;

	constructor(port: DataPort) {
		this.waiters = new Waiters(port);
	}

	onSettled(listener: (settled: Settled) => void) {
		if (this.settled !== undefined) listener(this.settled);
		else this.listeners.push(listener);
	}

	/** Yields until settled or `deadline`; undefined at the deadline. */
	wait(deadline?: number): Settled | undefined {
		const limit = deadline ?? math.huge;
		while (this.settled === undefined && bootClock() < limit) this.waiters.park(deadline);
		return this.settled;
	}

	settle(settled: Settled) {
		if (this.settled !== undefined) return;
		this.settled = settled;
		for (const listener of this.listeners) listener(settled);
		this.listeners.clear();
		this.waiters.wake();
	}
}

interface Entry {
	readonly request: Request;
	readonly ticket: Ticket;
	readonly lane: string;
	readonly sequence: number;
	readonly enqueuedAt: number;
	readonly counted: boolean;
	readyAt: number;
	retries: number;
}

interface Traffic {
	readonly at: number;
	readonly bytes: number;
	readonly update: boolean;
}

const enum Slot {
	Owned,
	Load,
	Ordinary,
}

function slotOf(priority: Priority): Slot {
	if (priority === Priority.Load) return Slot.Load;
	if (priority === Priority.Ordinary) return Slot.Ordinary;
	return Slot.Owned;
}

/** @internal The one keyed scheduler per server (rules 32 to 36). Every time is bootClock() (rule 25). */
export class KeyedScheduler {
	private readonly queue = new Array<Entry>();
	private readonly lanes = new Map<string, Entry>();
	private readonly traffic = new Map<string, Traffic[]>();
	private readonly pendingPerKey = new Map<string, number>();
	private pending = 0;
	private inFlight = 0;
	private shared = 0;
	private ordinary = 0;
	private sequence = 0;
	private cutoff?: number;
	private pumping = false;
	private again = false;
	private timer?: thread;
	private timerAt = math.huge;

	constructor(
		private readonly port: DataPort,
		private readonly diagnose: (key: string, message: string) => void,
	) {}

	/** Rule 33: against the documented per-server capacity, never against what remains. */
	reserve(): number {
		return math.max(
			BUDGET_RESERVE_MIN,
			math.floor(BUDGET_RESERVE_FRACTION * (60 + 40 * this.port.players.count())),
		);
	}

	setDispatchCutoff(at: number) {
		this.cutoff = at;
		this.pump();
	}

	submit(request: Request): Ticket {
		const ticket = new Ticket(this.port);
		const now = bootClock();
		if (this.cutoff !== undefined && now >= this.cutoff) {
			ticket.settle({ kind: "closing" });
			return ticket;
		}
		const lane = `${request.store}\0${request.key}`;
		const counted = slotOf(request.priority) !== Slot.Owned;
		if (counted) {
			const perKey = this.pendingPerKey.get(lane) ?? 0;
			if (this.pending >= MAX_PENDING_OPERATIONS || perKey >= MAX_PENDING_PER_KEY) {
				ticket.settle({ kind: "overloaded" });
				return ticket;
			}
			this.pending += 1;
			this.pendingPerKey.set(lane, perKey + 1);
		}
		this.queue.push({
			request,
			ticket,
			lane,
			sequence: ++this.sequence,
			enqueuedAt: now,
			counted,
			readyAt: now,
			retries: 0,
		});
		this.pump();
		return ticket;
	}

	private rank(entry: Entry, now: number): number {
		const request = entry.request;
		if (
			request.priority === Priority.Renew &&
			request.renewal === true &&
			(request.safeUntil ?? math.huge) - now < LEASE_CRITICAL_SECONDS
		)
			return Priority.LeaseCritical;
		if (request.priority === Priority.Ordinary && now - entry.enqueuedAt >= ORDINARY_AGING_SECONDS)
			return Priority.Receipt + 0.5;
		return request.priority;
	}

	private before(a: Entry, b: Entry, now: number): boolean {
		const ra = this.rank(a, now);
		const rb = this.rank(b, now);
		if (ra !== rb) return ra < rb;
		if (ra === Priority.LeaseCritical || ra === Priority.Release || ra === Priority.Renew) {
			const sa = a.request.safeUntil ?? math.huge;
			const sb = b.request.safeUntil ?? math.huge;
			if (sa !== sb) return sa < sb;
		}
		return a.sequence < b.sequence;
	}

	private hasSlot(entry: Entry): boolean {
		if (this.inFlight >= MAX_IN_FLIGHT) return false;
		const slot = slotOf(entry.request.priority);
		if (slot === Slot.Owned) return true;
		if (this.shared >= MAX_IN_FLIGHT - RESERVED_SESSION_SLOTS) return false;
		return slot === Slot.Load || this.ordinary < ORDINARY_MAX_IN_FLIGHT;
	}

	private hasBudget(entry: Entry): boolean {
		const update = entry.request.kind === "update";
		if (entry.request.priority !== Priority.Ordinary) return this.port.budget(entry.request.kind) >= 1;
		const reserve = this.reserve();
		if (this.port.budget("read") <= reserve) return false;
		return !update || this.port.budget("update") > reserve;
	}

	private recent(lane: string, now: number): Traffic[] {
		const kept = (this.traffic.get(lane) ?? []).filter((t) => now - t.at < 60);
		if (kept.size() === 0) this.traffic.delete(lane);
		else this.traffic.set(lane, kept);
		return kept;
	}

	private paced(entry: Entry, now: number): boolean {
		const update = entry.request.kind === "update";
		let bytes = entry.request.bytes;
		for (const t of this.recent(entry.lane, now)) if (t.update === update) bytes += t.bytes;
		return bytes <= (update ? KEY_WRITE_BYTES_PER_MINUTE : KEY_READ_BYTES_PER_MINUTE);
	}

	private settle(entry: Entry, settled: Settled) {
		const index = this.queue.indexOf(entry);
		if (index !== -1) this.queue.remove(index);
		if (this.lanes.get(entry.lane) === entry) this.lanes.delete(entry.lane);
		if (entry.counted) {
			this.pending -= 1;
			const perKey = (this.pendingPerKey.get(entry.lane) ?? 1) - 1;
			if (perKey <= 0) this.pendingPerKey.delete(entry.lane);
			else this.pendingPerKey.set(entry.lane, perKey);
		}
		entry.ticket.settle(settled);
	}

	private givenUp(entry: Entry): Settled {
		return entry.ticket.ambiguous ? { kind: "uncertain" } : { kind: "timeout" };
	}

	private finish(entry: Entry, outcome: StorageOutcome<StoredValue | undefined>) {
		entry.request.onResult?.(outcome);
		if (outcome.kind === "ok" || !outcome.retryable) {
			if (outcome.kind === "error")
				this.diagnose(
					`storage:${entry.request.store}:${outcome.code}`,
					`${entry.request.store} storage error ${outcome.code} on key ${entry.request.key}`,
				);
			this.settle(entry, { kind: "done", outcome });
			return;
		}
		if (outcome.ambiguous) entry.ticket.ambiguous = true;
		const now = bootClock();
		const span = math.min(RETRY_CAP_SECONDS, RETRY_BASE_SECONDS * 2 ** entry.retries);
		entry.retries += 1;
		entry.readyAt = now + this.port.random() * span;
		const limit = math.min(entry.request.dispatchDeadline, this.cutoff ?? math.huge);
		if (entry.readyAt >= limit) this.settle(entry, this.givenUp(entry));
		else this.queue.push(entry);
	}

	private dispatch(entry: Entry, now: number) {
		this.queue.remove(this.queue.indexOf(entry));
		this.lanes.set(entry.lane, entry);
		const slot = slotOf(entry.request.priority);
		this.inFlight += 1;
		if (slot !== Slot.Owned) this.shared += 1;
		if (slot === Slot.Ordinary) this.ordinary += 1;
		entry.ticket.dispatched = true;
		entry.ticket.inFlight = true;
		const request = entry.request;
		const update = request.kind === "update";
		const traffic = this.recent(entry.lane, now);
		traffic.push({ at: now, bytes: request.bytes, update });
		this.traffic.set(entry.lane, traffic);
		const transform = update ? request.transform!() : undefined;
		request.onDispatch?.(now);
		this.port.spawn(() => {
			const [ok, result] = pcall(() =>
				update
					? this.port.update(request.store, request.key, transform!)
					: this.port.read(request.store, request.key),
			);
			this.inFlight -= 1;
			if (slot !== Slot.Owned) this.shared -= 1;
			if (slot === Slot.Ordinary) this.ordinary -= 1;
			entry.ticket.inFlight = false;
			const outcome: StorageOutcome<StoredValue | undefined> = ok
				? result
				: { kind: "error", code: 0, retryable: true, ambiguous: true };
			const [handled, err] = pcall(() => this.finish(entry, outcome));
			if (!handled) {
				this.settle(entry, {
					kind: "done",
					outcome: { kind: "error", code: 0, retryable: false, ambiguous: true },
				});
				this.diagnose(`finish:${request.store}`, `${request.store} request handling failed: ${err}`);
			}
			this.pump();
		});
	}

	pump() {
		if (this.pumping) {
			this.again = true;
			return;
		}
		this.pumping = true;
		do {
			this.again = false;
			this.step();
		} while (this.again);
		this.pumping = false;
	}

	private step() {
		const now = bootClock();
		let wake = math.huge;
		let blocked = false;
		for (const entry of [...this.queue]) {
			const limit = math.min(entry.request.dispatchDeadline, this.cutoff ?? math.huge);
			if (now >= limit) this.settle(entry, this.givenUp(entry));
			else wake = math.min(wake, limit);
		}
		while (true) {
			let best: Entry | undefined;
			for (const entry of this.queue) {
				if (entry.readyAt > now) {
					wake = math.min(wake, entry.readyAt);
					continue;
				}
				const holder = this.lanes.get(entry.lane);
				if ((holder !== undefined && holder !== entry) || !this.hasSlot(entry)) continue;
				if (!this.hasBudget(entry) || !this.paced(entry, now)) {
					blocked = true;
					continue;
				}
				if (best === undefined || this.before(entry, best, now)) best = entry;
			}
			if (best === undefined) break;
			this.dispatch(best, now);
		}
		if (blocked) wake = math.min(wake, now + BUDGET_RECHECK_SECONDS);
		this.arm(wake, now);
	}

	private arm(at: number, now: number) {
		if (at === math.huge || at >= this.timerAt) return;
		if (this.timer !== undefined) this.port.cancel(this.timer);
		this.timerAt = at;
		this.timer = this.port.delay(math.max(at - now, 0), () => {
			this.timer = undefined;
			this.timerAt = math.huge;
			this.pump();
		});
	}
}
