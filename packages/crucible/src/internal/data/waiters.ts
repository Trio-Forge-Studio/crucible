import { bootClock } from "../boot";
import type { DataPort } from "./port";

/**
 * @internal Threads parked until a wake or their deadline. Callers re-check their condition in a loop, so a spurious
 * wake costs one check. A park id guards each resume, so a stale deadline timer never resumes a later park.
 */
export class Waiters {
	private readonly parked = new Map<thread, number>();
	private sequence = 0;

	constructor(private readonly port: DataPort) {}

	park(deadline?: number) {
		const thread = coroutine.running();
		const id = ++this.sequence;
		this.parked.set(thread, id);
		let fired = false;
		const timer =
			deadline === undefined
				? undefined
				: this.port.delay(math.max(deadline - bootClock(), 0), () => {
						fired = true;
						if (this.parked.get(thread) !== id) return;
						this.parked.delete(thread);
						this.port.resume(thread);
					});
		coroutine.yield();
		this.parked.delete(thread);
		if (timer !== undefined && !fired) this.port.cancel(timer);
	}

	/** Resumes in park order. */
	wake() {
		const threads = new Array<[thread, number]>();
		for (const [thread, id] of this.parked) threads.push([thread, id]);
		this.parked.clear();
		threads.sort((a, b) => a[1] < b[1]);
		for (const [thread] of threads) this.port.resume(thread);
	}
}
