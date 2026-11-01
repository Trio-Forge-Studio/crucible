import { CallResult, YIELDED } from "../runner";
import type { ObjectState } from "../state";
import { boundary, FrameKind, inBody, pop, push, top, type Completion, type Frame } from "./context";

/** @internal */
export const enum NodeKind {
	State,
	Computed,
	Effect,
	Lifetime,
}

/** @internal How far an observer is from trusting what it last read. */
export const enum Mark {
	Clean,
	/** Something upstream changed: validate dependencies in read order before trusting the cache. */
	Check,
	/** A direct dependency changed: recompute, or rerun an effect. */
	Dirty,
}

/** @internal */
export interface Edge {
	readonly source: Source;
	/** The source's version when the observer read it. */
	seen: number;
}

interface SourceFields {
	/** Creation order in the realm. Flush rounds run effects in ascending id. */
	readonly id: number;
	version: number;
	/** Attached observers only: effects, and computeds that have observers themselves. */
	readonly observers: Set<Observer>;
}

interface ObserverFields {
	readonly id: number;
	/** In read order. Replaced, never mutated, at the end of each evaluation or run. */
	dependencies: Edge[];
	mark: Mark;
}

/** @internal */
export interface StateNode extends SourceFields {
	readonly kind: NodeKind.State;
	value: unknown;
}

/** @internal */
export const enum CacheKind {
	Empty,
	Value,
	Error,
}

/** @internal */
export type Cache =
	| { readonly kind: CacheKind.Empty }
	| { readonly kind: CacheKind.Value; readonly value: unknown }
	| {
			readonly kind: CacheKind.Error;
			/** What was thrown. Two failures are the same outcome when their reasons are equal. */
			readonly reason: string;
			/** The reason with its traceback, rethrown to every reader. */
			readonly message: string;
	  };

/** @internal */
export interface ComputedNode extends SourceFields, ObserverFields {
	readonly kind: NodeKind.Computed;
	readonly read: () => unknown;
	readonly origin: string;
	cache: Cache;
	/** The epoch at the last validation. An unattached computed trusts its cache only while it is current. */
	validatedAt: number;
	/** `read` is running. Cleared at the end of every evaluation, including one cancelled for yielding. */
	evaluating: boolean;
	/** Set when this computed was read while evaluating; the evaluation caches it whatever `read` returns. */
	cycle?: string;
}

/** @internal The graph's view of an effect. The flush owns the rest of it. */
export interface EffectFields extends ObserverFields {
	readonly kind: NodeKind.Effect;
	stopped: boolean;
	queued: boolean;
}

/** @internal */
export type Source = StateNode | ComputedNode;
/** @internal */
export type Observer = ComputedNode | EffectFields;

let ids = 0;
/** @internal Creation order, shared by every node and effect in the realm. */
export function nextId(): number {
	ids += 1;
	return ids;
}

/** @internal Flush rounds sort by it, so an owner's effect runs before the effects its runs created. */
export function createdBefore(a: { readonly id: number }, b: { readonly id: number }): boolean {
	return a.id < b.id;
}

let epoch = 0;

/** @internal Moves on every value-changing write in the realm. */
export function currentEpoch(): number {
	return epoch;
}

let schedule: (effect: EffectFields) => void = () => {};

/** @internal The flush registers how a marked effect is queued. */
export function useScheduler(scheduler: (effect: EffectFields) => void) {
	schedule = scheduler;
}

/** @internal */
export function originOf(callback: Callback): string {
	const [source, line] = debug.info(callback, "sl");
	return `${source}:${line}`;
}

interface Failure {
	readonly reason: string;
	readonly message: string;
}

let thrown: Failure | undefined;

function capture(err: unknown): undefined {
	thrown = { reason: tostring(err), message: debug.traceback(tostring(err), 2) };
	return undefined;
}

/** @internal Throws when the running thread is inside a computed, where `what` is not allowed. */
export function guardPure(what: string) {
	const pure = top()?.pure;
	if (pure !== undefined) error(`${pure} must not ${what}`, 0);
}

function track(source: Source) {
	const frame = top();
	const observer = frame?.observer;
	if (observer === undefined || observer === source) return;
	const reads = frame!.reads!;
	if (reads.has(source)) return;
	reads.add(source);
	frame!.deps!.push({ source, seen: source.version });
}

/** @internal */
export function observerFrame(
	kind: FrameKind.Run | FrameKind.Pure,
	observer: Observer,
	lifetime?: Frame["lifetime"],
): Frame {
	return {
		kind,
		observer,
		lifetime,
		pure: kind === FrameKind.Pure ? "computed" : undefined,
		deps: [],
		reads: new Set(),
	};
}

/** @internal Calls `read` with no tracking. Ownership and purity stay those of the frame below. */
export function readUntracked<T>(read: () => T, pure?: string): T {
	const below = top();
	const frame: Frame = { kind: FrameKind.Untracked, lifetime: below?.lifetime, pure: pure ?? below?.pure };
	push(frame);
	const [ok, result] = pcall(read);
	pop(frame);
	if (!ok) error(result, 0);
	return result;
}

/** @internal */
export function createStateNode(value: unknown): StateNode {
	return { kind: NodeKind.State, id: nextId(), version: 0, observers: new Set(), value };
}

/** @internal */
export function readState(node: StateNode): unknown {
	track(node);
	return node.value;
}

/** @internal The one write path. Stores, bumps versions and marks; never runs user code or an effect. */
export function write(node: StateNode, value: unknown) {
	guardPure("write state");
	if (rawequal(value, node.value)) return;
	node.value = value;
	node.version += 1;
	epoch += 1;
	markObservers(node);
}

/** @internal A node and the value it will hold. */
export type PreparedWrite = readonly [node: StateNode, value: unknown];

/**
 * @internal
 * Rule 62's write phase for client apply: assignments, version and epoch bumps, and marks that only enqueue. No
 * purity guard, no admission and no user code, so nothing here can fail between two writes of one frame.
 */
export function commitPrepared(writes: readonly PreparedWrite[]) {
	for (const [node, value] of writes) {
		if (rawequal(value, node.value)) continue;
		node.value = value;
		node.version += 1;
		epoch += 1;
		markObservers(node);
	}
}

/** @internal */
export function update(node: StateNode, change: (current: unknown) => unknown) {
	write(
		node,
		readUntracked(() => change(node.value), "update's callback"),
	);
}

function markObservers(source: Source) {
	const pending = new Array<Observer>();
	const levels = new Array<Mark>();
	for (const o of source.observers) {
		pending.push(o);
		levels.push(Mark.Dirty);
	}
	while (pending.size() > 0) {
		const o = pending.pop()!;
		const level = levels.pop()!;
		if (o.mark >= level) continue;
		const wasClean = o.mark === Mark.Clean;
		o.mark = level;
		if (o.kind === NodeKind.Effect) schedule(o);
		else if (wasClean)
			for (const p of o.observers) {
				pending.push(p);
				levels.push(Mark.Check);
			}
	}
}

/** @internal */
export function createComputedNode(read: () => unknown): ComputedNode {
	return {
		kind: NodeKind.Computed,
		id: nextId(),
		version: 0,
		observers: new Set(),
		dependencies: [],
		mark: Mark.Dirty,
		read,
		origin: originOf(read),
		cache: { kind: CacheKind.Empty },
		validatedAt: -1,
		evaluating: false,
	};
}

/** @internal */
export function readComputed(node: ComputedNode): unknown {
	validate(node);
	track(node);
	const cache = node.cache;
	if (cache.kind === CacheKind.Error) error(cache.message, 0);
	return cache.kind === CacheKind.Value ? cache.value : undefined;
}

function cycle(node: ComputedNode): never {
	const message = `computed at ${node.origin} reads itself`;
	node.cycle = message;
	error(message, 0);
}

/** @internal After it returns, `node.cache` and `node.version` are current. */
export function validate(node: ComputedNode) {
	if (node.evaluating) cycle(node);
	if (node.cache.kind === CacheKind.Empty) return evaluate(node);
	if (node.observers.size() === 0) {
		if (node.mark === Mark.Clean && node.validatedAt === epoch) return;
		if (node.mark === Mark.Clean) node.mark = Mark.Check;
	}
	if (node.mark === Mark.Clean) return;
	if (node.mark === Mark.Check) {
		const at = epoch;
		let moved = false;
		for (const edge of node.dependencies) {
			const source = edge.source;
			if (source.kind === NodeKind.Computed) validate(source);
			if (source.version !== edge.seen) {
				moved = true;
				break;
			}
		}
		// A dependency evaluated in its own boundary can drain lifecycle work that writes; then recompute.
		if (!moved && epoch === at) {
			node.mark = Mark.Clean;
			node.validatedAt = at;
			return;
		}
	}
	evaluate(node);
}

interface Evaluation {
	readonly node: ComputedNode;
	readonly frame: Frame;
}

/** Runs `read` under a Pure frame on this thread. A yield suspends the thread; its boundary finishes the node. */
function evaluateInline(evaluation: Evaluation) {
	const { node, frame } = evaluation;
	push(frame);
	node.evaluating = true;
	thrown = undefined;
	const [ok, result] = xpcall(node.read, capture);
	pop(frame);
	const failure = ok ? undefined : (thrown ?? { reason: tostring(result), message: tostring(result) });
	thrown = undefined;
	settle(node, frame, result, failure);
}

function evaluate(node: ComputedNode) {
	const evaluation = { node, frame: observerFrame(FrameKind.Pure, node) };
	if (inBody()) evaluateInline(evaluation);
	else contained(undefined, evaluateInline, evaluation, () => {});
}

/** @internal Finishes every computed a body left open: its evaluation failed with the body's error. */
export function finishOpen(completion: Completion) {
	const message = tostring(completion.value);
	const failure = { reason: completion.result === CallResult.Yielded ? YIELDED : message, message };
	for (const frame of completion.open)
		if (frame.kind === FrameKind.Pure) settle(frame.observer as ComputedNode, frame, undefined, failure);
}

/** @internal `boundary` for graph bodies: computeds left open are finished before `complete` runs. */
export function contained<A>(
	owner: ObjectState | undefined,
	body: (arg: A) => unknown,
	arg: A,
	complete: (completion: Completion) => void,
) {
	boundary(owner, body, arg, (completion) => {
		finishOpen(completion);
		complete(completion);
	});
}

function settle(node: ComputedNode, frame: Frame, result: unknown, thrownFailure: Failure | undefined) {
	node.evaluating = false;
	let failure = thrownFailure;
	if (node.cycle !== undefined) {
		failure = { reason: node.cycle, message: node.cycle };
		node.cycle = undefined;
	}
	const reads = frame.deps!;
	let kept = reads;
	if (failure !== undefined) {
		kept = [...reads];
		for (const edge of node.dependencies)
			if (!frame.reads!.has(edge.source)) kept.push({ source: edge.source, seen: edge.source.version });
	}
	if (node.observers.size() > 0) rewire(node, node.dependencies, kept);
	node.dependencies = kept;
	const cache = node.cache;
	if (failure !== undefined) {
		if (cache.kind !== CacheKind.Error || cache.reason !== failure.reason) {
			node.cache = { kind: CacheKind.Error, reason: failure.reason, message: failure.message };
			node.version += 1;
		}
	} else if (cache.kind !== CacheKind.Value || !rawequal(cache.value, result)) {
		node.cache = { kind: CacheKind.Value, value: result };
		node.version += 1;
	}
	node.mark = Mark.Clean;
	node.validatedAt = epoch;
}

/** A computed that gains its first observer registers on its sources, transitively. */
function attach(root: ComputedNode) {
	const pending = [root];
	while (pending.size() > 0) {
		const node = pending.pop()!;
		if (node.mark === Mark.Clean && node.validatedAt !== epoch) node.mark = Mark.Check;
		for (const edge of node.dependencies) {
			const source = edge.source;
			const first = source.observers.size() === 0;
			source.observers.add(node);
			if (first && source.kind === NodeKind.Computed) pending.push(source);
		}
	}
}

/** A computed that loses its last observer leaves its sources, transitively, and keeps its dependency list. */
function detach(root: ComputedNode) {
	const pending = [root];
	while (pending.size() > 0) {
		const node = pending.pop()!;
		if (node.mark === Mark.Clean) node.validatedAt = epoch;
		for (const edge of node.dependencies) {
			const source = edge.source;
			if (!source.observers.has(node)) continue;
			source.observers.delete(node);
			if (source.kind === NodeKind.Computed && source.observers.size() === 0) pending.push(source);
		}
	}
}

function link(observer: Observer, source: Source) {
	const first = source.observers.size() === 0;
	source.observers.add(observer);
	if (first && source.kind === NodeKind.Computed) attach(source);
}

function unlink(observer: Observer, source: Source) {
	if (!source.observers.has(observer)) return;
	source.observers.delete(observer);
	if (source.kind === NodeKind.Computed && source.observers.size() === 0) detach(source);
}

/** @internal Moves an attached observer's registrations from `previous` to `following`. */
export function rewire(observer: Observer, previous: readonly Edge[], following: readonly Edge[]) {
	const had = new Set<Source>();
	for (const edge of previous) had.add(edge.source);
	const keep = new Set<Source>();
	for (const edge of following) {
		keep.add(edge.source);
		if (!had.has(edge.source)) link(observer, edge.source);
	}
	for (const edge of previous) if (!keep.has(edge.source)) unlink(observer, edge.source);
}

/** @internal Leaves every source. Used when an effect stops. */
export function releaseEdges(observer: EffectFields) {
	for (const edge of observer.dependencies) unlink(observer, edge.source);
	observer.dependencies = [];
}

/** @internal How far an effect is from current after a run: a source it read may have moved during the run. */
export function staleAfterRun(effect: EffectFields): Mark {
	let level = Mark.Clean;
	for (const edge of effect.dependencies) {
		const source = edge.source;
		if (source.version !== edge.seen) return Mark.Dirty;
		if (source.kind === NodeKind.Computed && source.mark !== Mark.Clean) level = Mark.Check;
	}
	return level;
}

/** @internal Raises an effect's mark and queues it. */
export function markEffect(effect: EffectFields, level: Mark) {
	if (effect.mark >= level) return;
	effect.mark = level;
	schedule(effect);
}

/** @internal Pulls an effect's computed dependencies current in read order. True when any of them moved. */
export function dependenciesMoved(effect: EffectFields): boolean {
	for (const edge of effect.dependencies) {
		const source = edge.source;
		if (source.kind === NodeKind.Computed) validate(source);
		if (source.version !== edge.seen) return true;
	}
	return false;
}
