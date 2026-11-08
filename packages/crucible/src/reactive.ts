import { createEffect, createScope, createWatch, registerCleanup, registerOwned } from "./internal/reactive/ownership";

/**
 * Keeps something outside Charm's graph in step with the atoms and computeds it reads. The body runs once now, and
 * again inside every write that changes what it last read, or once when the outermost batch ends. While boot is open
 * the first run waits for boot to commit, and never happens if boot rolls back. The body may return a cleanup, which
 * runs before the next run and when the effect ends. It belongs to the code that created it: inside `onEnable` it ends
 * at disable; otherwise it ends with its object, its parent effect run or its scope. Returns an idempotent dispose
 * that ends it sooner. A throw, a yield or another return value is thrown by Charm to the code that wrote, or here on
 * the first run. With no owner it throws.
 */
export function effect(body: () => void | (() => void)): () => void {
	return createEffect(body);
}

/**
 * Calls `onChange(value, previous)` on the runs where `source()` is not `rawequal` to the last value seen. The source
 * is read now, untracked, as the baseline. The source and `onChange` run as the watch's owner whoever wrote, and what
 * `onChange` creates ends at the next change. Owned, deferred during boot and disposed like `effect`.
 */
export function watch<T>(source: () => T, onChange: (value: T, previous: T) => void): () => void {
	return createWatch(source, onChange);
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

/**
 * What `own` releases: a function, called with no arguments; a connection, disconnected; an Instance, destroyed; a
 * thread, cancelled with `task.cancel`; or an object, through its `destroy()` or else its `Destroy()`.
 */
export type Ownable =
	| (() => void)
	| RBXScriptConnection
	| Instance
	| thread
	| { destroy(): void }
	| { Destroy(): void };

/**
 * Releases `value` once when the current owner ends, exactly as `onCleanup` would release a function that does it,
 * and returns `value`. How it is released is decided now, from its type. Under an owner that already ended it is
 * released at once. With no owner it throws, and so does a value that is not `Ownable`.
 */
export function own<T extends Ownable>(value: T): T {
	registerOwned(value);
	return value;
}
