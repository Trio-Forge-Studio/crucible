import type { ObjectState } from "./state";

/** @internal */
export const enum CallResult {
	Ok,
	Threw,
	Yielded,
}

/** @internal The error value of a call that yielded. */
export const YIELDED = "yielded; only start may yield";

type Work = (a: unknown, b: unknown) => unknown;

const DONE = {};
const free = new Array<thread>();
const owners = new Map<thread, ObjectState>();

function loop(work: Work | undefined, a: unknown, b: unknown) {
	while (true) {
		// The only resumer is `call`, which always passes a function and its two arguments.
		const result = (work as Work)(a, b);
		// A parked worker must not keep its last arguments alive.
		work = undefined;
		a = undefined;
		b = undefined;
		const [nextWork, nextA, nextB] = coroutine.yield(DONE, result);
		work = nextWork as Work;
		a = nextA;
		b = nextB;
	}
}

/**
 * @internal
 * Runs every callback that must not yield: hooks other than `start`, constructors, `accepts`, and `All<T>`
 * callbacks. A pool of parked worker coroutines (GoodSignal's free-runner trick) detects a yield, which
 * `pcall` cannot. A worker returns to the pool after a call that returned, so steady-state calls allocate
 * nothing. A throw kills its worker, and a yield gets its worker cancelled, so only failures allocate a new
 * one. A callback that calls `call` again takes another worker, and the pool grows to the deepest nesting.
 *
 * `owner` becomes the worker's owner for the call, which is what `current()` reads. Without one the call is
 * ownerless. Returns the callback's result on Ok, or the error (with a traceback) on Threw and Yielded.
 */
export function call<A, B>(
	work: (a: A, b: B) => unknown,
	a: A,
	b: B,
	owner?: ObjectState,
): LuaTuple<[result: CallResult, value: unknown]> {
	const worker = free.pop() ?? coroutine.create(loop);
	if (owner !== undefined) owners.set(worker, owner);
	const [ok, marker, value] = coroutine.resume(worker, work, a, b);
	owners.delete(worker);
	if (!ok) return $tuple(CallResult.Threw, debug.traceback(worker, tostring(marker)));
	if (marker !== DONE) {
		const trace = debug.traceback(worker, YIELDED);
		// A failed cancel leaves the worker suspended, so it never returns to the pool.
		const [cancelled, err] = pcall(task.cancel, worker);
		return $tuple(CallResult.Yielded, cancelled ? trace : `${trace}\ncancellation failed: ${err}`);
	}
	free.push(worker);
	return $tuple(CallResult.Ok, value);
}

/** @internal The object that owns `thread`: a runner worker during an owned call, or a start thread for its life. */
export function ownerOf(thread: thread): ObjectState | undefined {
	return owners.get(thread);
}

/** @internal */
export function own(thread: thread, owner: ObjectState) {
	owners.set(thread, owner);
}

/** @internal */
export function disown(thread: thread) {
	owners.delete(thread);
}
