import { closeScope, drain, openScope } from "../lifecycle";
import { call, CallResult } from "../runner";
import type { ObjectState } from "../state";
import type { Edge, Observer, Source } from "./graph";
import type { Lifetime } from "./ownership";

/** @internal */
export const enum FrameKind {
	/** An effect or watch run: tracks into its effect, owns into its run lifetime. */
	Run,
	/** A computed evaluation: tracks into the computed; writes and creation throw. */
	Pure,
	/** `untracked` or an `update` callback: no tracking; ownership and purity come from the frame below. */
	Untracked,
	/** A scope setup or a release: owns into its lifetime, tracks nothing. */
	Owning,
}

/** @internal What the running thread is doing for the graph. Only the thread that pushed a frame sees it. */
export interface Frame {
	readonly kind: FrameKind;
	readonly observer?: Observer;
	readonly lifetime?: Lifetime;
	/** Set inside a computed or an `update` callback, where writing and creating throw. Names it for the error. */
	readonly pure?: string;
	/** The observer's reads in order, deduplicated by `reads`. Set when `observer` is. */
	readonly deps?: Edge[];
	readonly reads?: Set<Source>;
}

const stacks = setmetatable(new Map<thread, Frame[]>(), { __mode: "k" });

/** @internal The innermost frame on the running thread. A spawned thread starts with none. */
export function top(): Frame | undefined {
	const stack = stacks.get(coroutine.running());
	return stack === undefined ? undefined : stack[stack.size() - 1];
}

/** @internal True on the worker of a body the graph runs, where a nested evaluation runs inline. */
export function inBody(): boolean {
	const stack = stacks.get(coroutine.running());
	return stack !== undefined && stack[0] !== undefined && stack[0].kind !== FrameKind.Untracked;
}

/** @internal */
export function push(frame: Frame) {
	const thread = coroutine.running();
	let stack = stacks.get(thread);
	if (stack === undefined) {
		stack = [];
		stacks.set(thread, stack);
	}
	stack.push(frame);
}

/** @internal Removes `frame` and anything left above it on the running thread, which keeps its emptied stack. */
export function pop(frame: Frame) {
	const stack = stacks.get(coroutine.running());
	if (stack === undefined) return;
	const index = stack.indexOf(frame);
	if (index < 0) return;
	while (stack.size() > index) stack.pop();
}

const NONE_OPEN: readonly Frame[] = [];

/**
 * @internal
 * Takes a finished worker's frames and returns the evaluations it left open, innermost first. A worker that returned
 * keeps its emptied stack for its next body.
 */
export function discard(worker: thread | undefined, returned: boolean): readonly Frame[] {
	if (worker === undefined) return NONE_OPEN;
	const stack = stacks.get(worker);
	if (stack === undefined) return NONE_OPEN;
	let open: Frame[] | undefined;
	for (let i = stack.size() - 1; i >= 0; i--) {
		const frame = stack[i];
		if (frame.kind !== FrameKind.Pure) continue;
		if (open === undefined) open = [];
		open.push(frame);
	}
	if (returned) stack.clear();
	else stacks.delete(worker);
	return open ?? NONE_OPEN;
}

/** @internal How a body ended, with the evaluations it left open, innermost first. */
export interface Completion {
	/** The frame the body ran under. */
	readonly frame: Frame | undefined;
	readonly result: CallResult;
	readonly value: unknown;
	readonly open: readonly Frame[];
	/** What the caller of `boundary` runs at completion, after its own `complete`. */
	readonly after: ((completion: Completion) => void) | undefined;
}

/** A body on its way through `boundary`. It becomes its own completion. */
interface Job {
	readonly frame: Frame | undefined;
	readonly body: (arg?: unknown) => unknown;
	readonly arg: unknown;
	readonly after: ((completion: Completion) => void) | undefined;
	worker?: thread;
	result: CallResult;
	value: unknown;
	open: readonly Frame[];
}

/** Starts the worker's stack at the body's frame. */
function inner(job: Job) {
	const worker = coroutine.running();
	job.worker = worker;
	const frame = job.frame;
	if (frame !== undefined) {
		const stack = stacks.get(worker);
		if (stack === undefined || stack.size() > 0) stacks.set(worker, [frame]);
		else stack.push(frame);
	}
	return job.arg === undefined ? job.body() : job.body(job.arg);
}

function traceback(err: unknown) {
	return debug.traceback(tostring(err), 2);
}

/**
 * @internal
 * The completion boundary. The body runs under `frame` on a pooled worker inside a lifecycle scope, and `complete`
 * runs on this thread before the scope closes, so whatever the body left behind is finished before its requests
 * drain. `complete` runs framework code only. A failure there is an internal error and is rethrown here.
 */
export function boundary<A>(
	owner: ObjectState | undefined,
	frame: Frame | undefined,
	body: (arg: A) => unknown,
	arg: A,
	complete: (completion: Completion) => void,
	after?: (completion: Completion) => void,
) {
	const job: Job = {
		frame,
		body: body as (arg?: unknown) => unknown,
		arg,
		after,
		result: CallResult.Ok,
		value: undefined,
		open: NONE_OPEN,
	};
	openScope();
	const [result, value] = call(inner, job, undefined, owner);
	job.result = result;
	job.value = value;
	job.open = discard(job.worker, result === CallResult.Ok);
	const [ok, err] = xpcall(complete, traceback, job);
	if (closeScope()) drain();
	if (!ok) error(err, 0);
}
