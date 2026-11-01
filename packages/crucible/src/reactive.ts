import { createEffect, createWatch } from "./internal/reactive/flush";
import {
	createComputedNode,
	createStateNode,
	readComputed,
	readState,
	readUntracked,
	update,
	write,
	type StateNode,
} from "./internal/reactive/graph";
import { createScope, registerCleanup } from "./internal/reactive/ownership";

declare const stateKind: unique symbol;

/**
 * A value read by calling it. Any function of no arguments is one, so a getter that reads state, such as
 * `() => this.open() && !this.locked()`, tracks like a state or a computed.
 */
export interface Readable<T> {
	(): T;
}

/**
 * A writable local value. `set` stores its argument as is, and `update` stores what its callback returns for the
 * current value. A write equal (`rawequal`) to the current value does nothing, so replace tables instead of
 * mutating them. At runtime a State is a callable table, not a function: pass `() => s()` to code that tests
 * `type(x) == "function"`.
 */
export interface State<T> extends Readable<T> {
	readonly [stateKind]: { readonly value: (value: T) => T };
	set(value: T): void;
	update(change: (current: T) => T): void;
}

const stateMethods = {
	set(this: StateNode, value: unknown) {
		write(this, value);
	},
	update(this: StateNode, change: (current: unknown) => unknown) {
		update(this, change);
	},
};

const stateMeta: LuaMetatable<StateNode> = {
	__index: stateMethods as never,
	__call: (node) => readState(node),
};

/** A local source of truth. It needs no owner: a field initializer, a local or a module constant. */
export function state<T>(initial: T): State<T> {
	return setmetatable(createStateNode(initial), stateMeta) as unknown as State<T>;
}

/**
 * A cached value derived from other readables. It evaluates on its first read and again only when read after a
 * dependency changed. A result equal to the cached one stops propagation. A throw or a yield is cached and rethrown
 * to every reader until a dependency changes. Writing state or creating an effect, watch, scope or cleanup inside
 * `read` throws. It needs no owner.
 */
export function computed<T>(read: () => T): Readable<T> {
	const node = createComputedNode(read);
	return () => readComputed(node) as T;
}

/**
 * Keeps something outside the graph in step with state. The body runs on the next reactive flush, never inside
 * the call that created it, and again after anything it read changed. It may return a cleanup, which runs before
 * the next run and when the effect ends. It belongs to the code that created it: inside `onEnable` it ends at
 * disable; otherwise it ends with its object, its parent effect run or its scope. A throw, a yield or another
 * return value stops it for good, after what its run created is released. With no owner it throws.
 */
export function effect(body: () => void | (() => void)): void {
	createEffect(body);
}

/**
 * Calls `onChange(value, previous)` on the flushes where `source()` differs from the last value seen. The source is
 * read now as the baseline, so a change before the first flush is seen. It observes the value at each flush, not
 * each write. Owned and stopped like `effect`.
 */
export function watch<T>(source: Readable<T>, onChange: (value: T, previous: T) => void): void {
	createWatch(source, onChange);
}

/** Calls `read` without tracking what it reads. Ownership is unchanged. */
export function untracked<T>(read: () => T): T {
	return readUntracked(read);
}

/**
 * Runs `setup` now as a new lifetime that owns every effect, watch, scope and cleanup created inside it. Returns
 * an idempotent dispose. Inside another owner it also ends when that owner ends; outside every owner it is a root
 * that only its dispose ends. If `setup` throws or yields, what it created ends and the error is rethrown.
 */
export function scope(setup: () => void): () => void {
	return createScope(setup);
}

/**
 * Registers `release` to run once when the current owner ends: after the owner's effects and scopes ended, newest
 * release first. Under an owner that already ended it runs at once. With no owner it throws.
 */
export function onCleanup(release: () => void): void {
	registerCleanup(release);
}
