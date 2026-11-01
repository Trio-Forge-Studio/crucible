import { within } from "../lifecycle";
import { call, CallResult } from "../runner";
import { Hook, type ObjectState } from "../state";
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

/** @internal Removes `frame` and anything left above it on the running thread. */
export function pop(frame: Frame) {
	const thread = coroutine.running();
	const stack = stacks.get(thread);
	if (stack === undefined) return;
	const index = stack.indexOf(frame);
	if (index < 0) return;
	while (stack.size() > index) stack.pop();
	if (stack.size() === 0) stacks.delete(thread);
}

function discard(worker: thread): Frame[] {
	const stack = stacks.get(worker);
	stacks.delete(worker);
	const open = new Array<Frame>();
	if (stack !== undefined) for (let i = stack.size() - 1; i >= 0; i--) open.push(stack[i]);
	return open;
}

/** @internal How a body ended, with the frames it left open, innermost first. */
export interface Completion {
	readonly result: CallResult;
	readonly value: unknown;
	readonly open: readonly Frame[];
}

interface Job {
	readonly body: (arg: unknown) => unknown;
	readonly arg: unknown;
	readonly complete: (completion: Completion) => void;
	worker?: thread;
}

function inner(job: Job) {
	job.worker = coroutine.running();
	return job.body(job.arg);
}

function outer(job: Job, owner: ObjectState | undefined) {
	const [result, value] = call(inner, job, undefined, owner);
	const open = job.worker === undefined ? [] : discard(job.worker);
	job.complete({ result, value, open });
}

/**
 * @internal
 * The completion boundary. `within` runs `outer` on one worker, which runs the body on a second worker and then
 * completes it, so whatever the body left behind is finished before `within` drains lifecycle requests. `outer`
 * runs framework code only. A failure there is an internal error and is rethrown here.
 */
export function boundary<A>(
	owner: ObjectState | undefined,
	body: (arg: A) => unknown,
	arg: A,
	complete: (completion: Completion) => void,
) {
	const job: Job = { body: body as (arg: unknown) => unknown, arg, complete };
	const [result, value] = within(owner, undefined, Hook.Callback, outer, job, owner);
	if (result !== CallResult.Ok) error(value, 0);
}
