import { runtimeDiagnostic } from "../diagnostics";
import { closeScope, current, drain, enabling, hasEnded, openScope } from "../lifecycle";
import type { ClassRecord } from "../registry";
import { call, CallResult } from "../runner";
import type { ObjectState } from "../state";
import { boundary, discard, FrameKind, pop, push, top, type Completion, type Frame } from "./context";
import { finishOpen, guardPure, NodeKind, originOf, releaseEdges, type EffectFields } from "./graph";
import { OWNER_CHILDREN_WARNING } from "./limits";

/** @internal */
export const enum Role {
	/** One Crucible generation. Ends through `ObjectState.subscriptions`, after `onDestroy`. */
	Generation,
	/** A behaviour's enable interval. Ends right after `onDisable`, or when the generation is released. */
	Interval,
	/** One run of an effect. Ends before the next run, or when the effect stops. */
	Run,
	/** `scope(setup)`: a child of its owner, or a root. */
	Scope,
}

/**
 * @internal
 * How a lifetime releases one owned value. `own` decides it once, when the value is owned. The engine's kinds come
 * last, so `kind >= Release.Disconnect` is a release that runs no Luau.
 */
export const enum Release {
	Call,
	/** A table's `destroy()`. */
	DestroyMethod,
	/** A table's `Destroy()`. */
	DestroyUpperMethod,
	Disconnect,
	DestroyInstance,
	Cancel,
}

interface Destroyable {
	destroy(): void;
}

interface UpperDestroyable {
	Destroy(): void;
}

interface DestroyFields {
	readonly destroy?: unknown;
	readonly Destroy?: unknown;
}

interface Linked {
	/** The lifetime that owns it. A run's parent is its effect's owner; a run is not in any child list. */
	readonly parent?: Lifetime;
	prev?: Owned;
	next?: Owned;
	linked: boolean;
}

/** @internal Everything that must end, ends exactly once, through one of these. */
export interface Lifetime extends Linked {
	readonly kind: NodeKind.Lifetime;
	readonly role: Role;
	/** The Crucible generation at the root of the chain, which boundaries pass to `within`. None for a root. */
	readonly owner?: ObjectState;
	/** The effect a Run lifetime belongs to. */
	readonly effect?: EffectNode;
	head?: Owned;
	tail?: Owned;
	count: number;
	/** What it releases when it ends, oldest first, as pairs of a `Release` and its value. */
	readonly releases: unknown[];
	/** Latched first when it starts ending: nothing new attaches and no queued run of its effects starts. */
	ended: boolean;
	/** Its children and releases have been ended. */
	finished: boolean;
	/** Endings that wait for this body's completion, because they reached it while it ran. */
	deferred?: Lifetime[];
	warned: boolean;
	/** Inherited context for the UI layer. Unused in v1. */
	context?: Map<unknown, unknown>;
}

/** @internal */
export interface EffectNode extends EffectFields, Linked {
	readonly parent: Lifetime;
	readonly body: () => unknown;
	readonly origin: string;
	/** The current run's lifetime: what the run created. */
	run?: Lifetime;
	/** Runs in the current flush. */
	runs: number;
}

type Owned = Lifetime | EffectNode;

/** @internal */
export const NO_OWNER = "effect() needs an owner: create it in a Subsystem or behaviour, or inside scope()";

const generations = setmetatable(new Map<ObjectState, Lifetime>(), { __mode: "k" });
const intervals = setmetatable(new Map<ObjectState, Lifetime>(), { __mode: "k" });
/** Bodies running now, outermost first. Boundaries nest synchronously, so one stack serves every thread. */
const bodies = new Array<Lifetime>();

let describe: (instance: Instance) => string = tostring;

/** @internal */
export function useDescribe(port: (instance: Instance) => string) {
	describe = port;
}

/** @internal "JobService" or "DoorPrompt on Workspace.Door", for diagnostics. */
export function ownerLabel(owner: ObjectState | undefined): string {
	if (owner === undefined) return "a root scope";
	const id = (owner.record as ClassRecord).id;
	return owner.instance === undefined ? id : `${id} on ${describe(owner.instance)}`;
}

/** @internal */
export function newLifetime(
	role: Role,
	parent: Lifetime | undefined,
	owner: ObjectState | undefined,
	effect?: EffectNode,
) {
	const lifetime: Lifetime = {
		kind: NodeKind.Lifetime,
		role,
		parent,
		owner,
		effect,
		count: 0,
		releases: [],
		ended: false,
		finished: false,
		linked: false,
		warned: false,
	};
	return lifetime;
}

/** @internal Appends `child` to `parent`'s children, newest last. */
export function adopt(parent: Lifetime, child: Owned) {
	child.prev = parent.tail;
	child.next = undefined;
	if (parent.tail === undefined) parent.head = child;
	else parent.tail.next = child;
	parent.tail = child;
	child.linked = true;
	parent.count += 1;
	if (parent.role === Role.Generation && parent.count >= OWNER_CHILDREN_WARNING && !parent.warned) {
		parent.warned = true;
		runtimeDiagnostic(
			"warning",
			`Crucible: ${ownerLabel(parent.owner)} owns ${OWNER_CHILDREN_WARNING} live effects and scopes; one may be created every frame`,
		);
	}
}

function leave(child: Owned) {
	const parent = child.parent;
	if (!child.linked || parent === undefined) return;
	child.linked = false;
	if (child.prev === undefined) parent.head = child.next;
	else child.prev.next = child.next;
	if (child.next === undefined) parent.tail = child.prev;
	else child.next.prev = child.prev;
	child.prev = undefined;
	child.next = undefined;
	parent.count -= 1;
}

function generation(s: ObjectState): Lifetime {
	let lifetime = generations.get(s);
	if (lifetime === undefined) {
		const created = newLifetime(Role.Generation, undefined, s);
		lifetime = created;
		generations.set(s, created);
		if (s.subscriptions === undefined) s.subscriptions = [];
		s.subscriptions.push(() => {
			generations.delete(s);
			endLifetime(created);
		});
	}
	return lifetime;
}

function interval(s: ObjectState): Lifetime {
	let lifetime = intervals.get(s);
	if (lifetime === undefined) {
		const parent = generation(s);
		lifetime = newLifetime(Role.Interval, parent, s);
		adopt(parent, lifetime);
		intervals.set(s, lifetime);
	}
	return lifetime;
}

/**
 * @internal
 * The owner of something created now: the innermost frame's lifetime; else the running Crucible object's enable
 * interval while its Enable step runs, or its generation; else none. A returned lifetime may have ended.
 */
export function resolveOwner(): Lifetime | undefined {
	return ownerAt(top());
}

function ownerAt(frame: Frame | undefined): Lifetime | undefined {
	if (frame?.lifetime !== undefined) return frame.lifetime;
	const s = current();
	if (s === undefined) return undefined;
	if (hasEnded(s)) {
		const ended = newLifetime(Role.Generation, undefined, s);
		ended.ended = true;
		ended.finished = true;
		return ended;
	}
	return enabling() === s ? interval(s) : generation(s);
}

/** @internal Core edit E1: lifecycle calls it after `onDisable` and when it releases a generation. Idempotent. */
export function endInterval(s: ObjectState) {
	const lifetime = intervals.get(s);
	if (lifetime === undefined) return;
	intervals.delete(s);
	endLifetime(lifetime);
}

/** @internal Detaches an effect from the graph for good. No diagnostic: an ending is not a failure. */
function halt(effect: EffectNode) {
	if (effect.stopped) return;
	effect.stopped = true;
	releaseEdges(effect);
}

function latch(root: Lifetime) {
	if (root.head === undefined) {
		root.ended = true;
		return;
	}
	const pending = [root];
	while (pending.size() > 0) {
		const lifetime = pending.pop()!;
		lifetime.ended = true;
		let child = lifetime.head;
		while (child !== undefined) {
			if (child.kind === NodeKind.Effect) {
				halt(child);
				if (child.run !== undefined && !child.run.ended) pending.push(child.run);
			} else if (!child.ended) pending.push(child);
			child = child.next;
		}
	}
}

function under(body: Lifetime, lifetime: Lifetime): boolean {
	let at: Lifetime | undefined = body;
	while (at !== undefined) {
		if (at === lifetime) return true;
		at = at.parent;
	}
	return false;
}

/**
 * @internal
 * Ends a lifetime once: latch it and everything under it, then, unless a body under it is still running, end its
 * children newest first, run its releases newest first, and unlink it. A running body defers those steps to its
 * completion.
 */
export function endLifetime(lifetime: Lifetime) {
	if (lifetime.ended) return;
	latch(lifetime);
	for (const body of bodies) {
		if (!under(body, lifetime)) continue;
		if (body.deferred === undefined) body.deferred = [];
		body.deferred.push(lifetime);
		return;
	}
	finish(lifetime);
}

function finish(lifetime: Lifetime) {
	if (lifetime.finished) return;
	lifetime.finished = true;
	let child = lifetime.tail;
	while (child !== undefined) {
		const previous: Owned | undefined = child.prev;
		if (child.kind === NodeKind.Effect) retire(child);
		else finish(child);
		child = previous;
	}
	const releases = lifetime.releases;
	runReleases(lifetime, releases, releases.size() - 2);
	releases.clear();
	leave(lifetime);
}

/** @internal Stops an effect for good: detached, its current run ended, unlinked from its owner. */
export function retire(effect: EffectNode) {
	halt(effect);
	const run = effect.run;
	effect.run = undefined;
	if (run !== undefined) {
		latch(run);
		finish(run);
	}
	leave(effect);
}

function releaseLabel(kind: Release, value: unknown): string {
	if (kind === Release.Disconnect) return "of a connection";
	if (kind === Release.DestroyInstance) return `of ${describe(value as Instance)}`;
	if (kind === Release.Cancel) return "of a thread";
	const code =
		kind === Release.Call
			? value
			: kind === Release.DestroyMethod
				? (value as DestroyFields).destroy
				: (value as DestroyFields).Destroy;
	return typeIs(code, "function") ? `at ${originOf(code)}` : `of ${tostring(value)}`;
}

function reportRelease(lifetime: Lifetime, kind: Release, value: unknown, err: unknown) {
	runtimeDiagnostic(
		"error",
		`Crucible: cleanup ${releaseLabel(kind, value)} (${ownerLabel(lifetime.owner)}) failed: ${tostring(err)}`,
	);
}

function disconnect(connection: RBXScriptConnection) {
	connection.Disconnect();
}

function destroyInstance(instance: Instance) {
	instance.Destroy();
}

/**
 * `Disconnect`, `Destroy` and `task.cancel` run no Luau on this thread and cannot yield, so a `pcall` is their
 * boundary. A throw is reported and nobody is faulted.
 */
function releaseEngine(lifetime: Lifetime, kind: Release, value: unknown) {
	let engine: (value: never) => void;
	if (kind === Release.Disconnect) engine = disconnect;
	else if (kind === Release.DestroyInstance) engine = destroyInstance;
	else engine = task.cancel;
	const [ok, err] = pcall(engine, value as never);
	if (!ok) pcall(reportRelease, lifetime, kind, value, err);
}

/** A lifetime's releases that run code, newest first, on one worker until one fails or yields. Its own frame. */
interface Batch extends Frame {
	readonly kind: FrameKind.Owning;
	readonly lifetime: Lifetime;
	readonly releases: readonly unknown[];
	/** The index of the release running now. */
	at: number;
	/** That release's lifecycle scope is open. */
	scoped: boolean;
	worker?: thread;
}

/**
 * Runs `batch` from `batch.at` down on the running worker, as its ended lifetime. A release that runs code opens its
 * own lifecycle scope, as `within` would. One that leaves requests for a drain outside any other scope stops the
 * batch with `true`, so the caller drains before the next release, as that release's own `within` would have.
 */
function releaseBatch(batch: Batch): boolean {
	batch.worker = coroutine.running();
	const lifetime = batch.lifetime;
	const releases = batch.releases;
	push(batch);
	for (let i = batch.at; i >= 0; i -= 2) {
		batch.at = i;
		const kind = releases[i] as Release;
		const value = releases[i + 1];
		if (kind >= Release.Disconnect) {
			releaseEngine(lifetime, kind, value);
			continue;
		}
		batch.scoped = true;
		openScope();
		if (kind === Release.Call) (value as () => void)();
		else if (kind === Release.DestroyMethod) (value as Destroyable).destroy();
		else (value as UpperDestroyable).Destroy();
		batch.scoped = false;
		if (closeScope()) {
			pop(batch);
			return true;
		}
	}
	pop(batch);
	return false;
}

/** The release at `batch.at` threw or yielded, and its worker is gone: finish what it left open and report it. */
function failRelease(batch: Batch, result: CallResult, value: unknown) {
	finishOpen({ frame: undefined, result, value, open: discard(batch.worker, false), after: undefined });
	const at = batch.at;
	reportRelease(batch.lifetime, batch.releases[at] as Release, batch.releases[at + 1], value);
	if (!batch.scoped) return;
	batch.scoped = false;
	if (closeScope()) drain();
}

/**
 * Releases `releases` from index `from` down, as their ended lifetime. Engine values before the first function or
 * `destroy()` release here. From there one worker runs the rest as a batch. A release that throws or yields loses
 * that worker: it is cancelled and reported, and the batch goes on from the next release on a fresh one. Each
 * release's requests still apply when it returns, before the next one runs.
 */
function runReleases(lifetime: Lifetime, releases: readonly unknown[], from: number) {
	let batch: Batch | undefined;
	let i = from;
	while (i >= 0) {
		const kind = releases[i] as Release;
		if (kind >= Release.Disconnect) {
			releaseEngine(lifetime, kind, releases[i + 1]);
			i -= 2;
			continue;
		}
		if (batch === undefined) batch = { kind: FrameKind.Owning, lifetime, releases, at: i, scoped: false };
		else batch.at = i;
		const [result, value] = call(releaseBatch, batch, undefined, lifetime.owner);
		if (result !== CallResult.Ok) failRelease(batch, result, value);
		else if (value === true) drain();
		i = batch.at - 2;
	}
}

/** Releases one value now, as its ended lifetime: a throw or a yield is reported and nobody is faulted. */
function releaseValue(lifetime: Lifetime, kind: Release, value: unknown) {
	if (kind >= Release.Disconnect) releaseEngine(lifetime, kind, value);
	else runReleases(lifetime, [kind, value], 0);
}

/** @internal */
export function hold(lifetime: Lifetime, kind: Release, value: unknown) {
	const releases = lifetime.releases;
	const size = releases.size();
	releases[size] = kind;
	releases[size + 1] = value;
}

/**
 * @internal
 * Runs a lifetime's own body in a boundary, under `frame`, whose lifetime it is. While it runs, endings that reach
 * it wait; at completion `after` runs, then those endings.
 */
export function runBody<A>(frame: Frame, body: (arg: A) => unknown, arg: A, after: (completion: Completion) => void) {
	const lifetime = frame.lifetime!;
	bodies.push(lifetime);
	boundary(lifetime.owner, frame, body, arg, completeBody, after);
}

function settleBody(completion: Completion) {
	finishOpen(completion);
	completion.after!(completion);
}

function completeBody(completion: Completion) {
	const lifetime = completion.frame!.lifetime!;
	const index = bodies.indexOf(lifetime);
	if (index >= 0) bodies.remove(index);
	const [ok, err] = pcall(settleBody, completion);
	const deferred = lifetime.deferred;
	lifetime.deferred = undefined;
	if (deferred !== undefined) for (const ending of deferred) finish(ending);
	if (!ok) error(err, 0);
}

/** @internal */
export function createScope(setup: () => void): () => void {
	guardPure("create scopes");
	const parent = resolveOwner();
	if (parent?.ended) return () => {};
	return startScope(parent, setup);
}

/** @internal A scope that only its dispose ends, whatever owner is current. */
export function createRootScope(setup: () => void): () => void {
	guardPure("create scopes");
	return startScope(undefined, setup);
}

function startScope(parent: Lifetime | undefined, setup: () => void): () => void {
	const scope = newLifetime(Role.Scope, parent, parent?.owner);
	if (parent !== undefined) adopt(parent, scope);
	let failed = false;
	let failure: unknown;
	runBody({ kind: FrameKind.Owning, lifetime: scope }, setup, undefined, (completion) => {
		if (completion.result === CallResult.Ok) return;
		failed = true;
		failure = completion.value;
		endLifetime(scope);
	});
	if (failed) error(failure, 0);
	return () => endLifetime(scope);
}

/** `guardPure("register cleanups")` that returns the frame it read, so `register` does not read it again. */
function cleanupFrame(): Frame | undefined {
	const frame = top();
	const pure = frame?.pure;
	if (pure !== undefined) error(`${pure} must not register cleanups`, 0);
	return frame;
}

function register(frame: Frame | undefined, kind: Release, value: unknown) {
	const owner = ownerAt(frame);
	if (owner === undefined) error(NO_OWNER, 0);
	if (owner.ended) releaseValue(owner, kind, value);
	else hold(owner, kind, value);
}

/** @internal */
export function registerCleanup(release: () => void) {
	const frame = cleanupFrame();
	// A nil from untyped Luau would shift every pair after it, so it is dropped.
	if (release === undefined) return;
	register(frame, Release.Call, release);
}

const NOT_OWNABLE =
	"own() takes a function, an RBXScriptConnection, an Instance, a thread, or an object with destroy() or Destroy()";

function classify(value: unknown): Release {
	const typeName = typeOf(value);
	if (typeName === "RBXScriptConnection") return Release.Disconnect;
	if (typeName === "Instance") return Release.DestroyInstance;
	if (typeName === "function") return Release.Call;
	if (typeName === "thread") return Release.Cancel;
	if (typeName === "table") {
		if (typeIs((value as DestroyFields).destroy, "function")) return Release.DestroyMethod;
		if (typeIs((value as DestroyFields).Destroy, "function")) return Release.DestroyUpperMethod;
	}
	error(`${NOT_OWNABLE}; got a ${typeName}`, 0);
}

/** @internal */
export function registerOwned(value: unknown) {
	const frame = cleanupFrame();
	register(frame, classify(value), value);
}
