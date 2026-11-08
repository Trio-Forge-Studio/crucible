import {
	effect as charmEffect,
	effectScope,
	getActiveSub,
	onCleanup as charmOnCleanup,
	ReactiveFlags,
	setActiveSub,
	signal,
	startBatch,
	untracked,
	type Getter,
	type ReactiveNode,
	type Setter,
} from "@rbxts/charm";
import { runtimeDiagnostic } from "../diagnostics";
import { closeScope, drain, enabling, hasEnded, openScope } from "../lifecycle";
import type { ClassRecord } from "../registry";
import { call, CallResult, running } from "../runner";
import type { ObjectState } from "../state";
import { peek, settle } from "./store";

/** @internal */
export const enum Role {
	/** One Crucible generation. Ends through `ObjectState.subscriptions`, after `onDestroy`. */
	Generation,
	/** A behaviour's enable interval. Ends right after `onDisable`, or when the generation is released. */
	Interval,
	/** One Crucible effect. Its releases are its current run's, and end before the next run. */
	Run,
	/** `scope(setup)`: a child of its owner, or a root. */
	Scope,
}

/**
 * @internal
 * How an owner releases one owned value. `own` decides it once, when the value is owned. The engine's kinds come
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

/**
 * @internal
 * What effects, scopes and releases are created into. A generation, an interval or a scope is a detached Charm effect
 * scope, whose one cleanup ends the owner. A Run is one Crucible effect: its node is the Charm effect, and it arms a
 * cleanup on that effect for each run that holds a release or a child scope.
 *
 * An owner lists the scopes, and the effects outside every run, that it ends, instead of linking them into its Charm
 * node: Charm 0.11.1's `stopEffect` unlinks a linked node from the wrong parent, which corrupts the parent's
 * dependencies when the node is disposed before it. An effect created inside a run is linked into the running Charm
 * effect, as Charm nests it, and is never stopped: ending it writes its `closed` signal, so Charm reruns it with no
 * body and drops what it read, the effects it created included.
 */
export interface Owner {
	readonly role: Role;
	/** The Charm node: a scope-like owner's scope, or a Run's effect once its first run started. */
	node: ReactiveNode;
	/** The Crucible generation at the root of the chain: what `current()` reports inside it, and diagnostics name. */
	readonly state: ObjectState | undefined;
	/** Latched when it starts ending, and for a Run while its releases run. */
	ended: boolean;
	/** What it releases when it ends, as pairs of a `Release` and its value, oldest first. */
	releases?: unknown[];
	/** Stops its Charm node: a scope-like owner's scope, or a listed Run's effect. */
	stop?: () => void;
	/** A Run's body. Disposing the Run clears it, and a run without one reads nothing, so Charm drops it. */
	body?: () => unknown;
	/** A Run's cleanup, made once and armed on each run that holds a release or a child scope. */
	finish?: () => void;
	/** A Run's cleanup is armed on the run in progress. */
	armed?: boolean;
	/** A nested Run's signal, read by each of its runs, that its end writes. */
	closed?: Getter<boolean>;
	close?: Setter<boolean>;
	/** A Run's first run is in progress inside `create`, which throws its failure once Charm returned the stop. */
	creating?: boolean;
	failure?: unknown;
	/** The owner whose list holds this one. */
	parent?: Owner;
	/** What it ends before its releases, oldest first, linked through `prev` and `next`. */
	first?: Owner;
	last?: Owner;
	prev?: Owner;
	next?: Owner;
}

/** @internal */
export const NO_OWNER = "effect() needs an owner: create it in a Subsystem or behaviour, or inside scope()";

const generations = setmetatable(new Map<ObjectState, Owner>(), { __mode: "k" });
/** Nested Runs by their Charm effect, so ending one finds the Runs it created among its dependencies. */
const nestedRuns = setmetatable(new Map<ReactiveNode, Owner>(), { __mode: "kv" });
const intervals = setmetatable(new Map<ObjectState, Owner>(), { __mode: "k" });

/** Listed Runs created while boot is open, in creation order, waiting for the commit to create their effects. */
let pending: Owner[] | undefined;

let describe: (instance: Instance) => string = tostring;

/** @internal Boot installs the engine's describer, so a failed release names its Instance. */
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
export function originOf(callback: Callback): string {
	const [source, line] = debug.info(callback, "sl");
	return `${source}:${line}`;
}

/** @internal An owner that has ended takes nothing new: a Run has when its Charm effect has stopped. */
export function isEnded(owner: Owner): boolean {
	return (
		owner.ended || (owner.role === Role.Run && owner.node !== undefined && owner.node.flags === ReactiveFlags.None)
	);
}

/** Ends what an owner lists, oldest first, then runs the releases it holds, newest first, once. */
function finish(owner: Owner) {
	owner.ended = true;
	owner.armed = undefined;
	// A child's end can end any sibling, so each turn takes the head of the list as it is then.
	let child = owner.first;
	while (child !== undefined) {
		unlist(child);
		dispose(child);
		child = owner.first;
	}
	const releases = owner.releases;
	owner.releases = undefined;
	if (releases !== undefined) runReleases(owner, releases, releases.size() - 2);
}

/** Lists `child` last on `parent`, which ends it. A Run lists only for its current run. */
function adopt(parent: Owner, child: Owner) {
	if (parent.role === Role.Run) arm(parent);
	child.parent = parent;
	child.prev = parent.last;
	if (parent.last === undefined) parent.first = child;
	else parent.last.next = child;
	parent.last = child;
}

function unlist(owner: Owner) {
	const parent = owner.parent;
	if (parent === undefined) return;
	owner.parent = undefined;
	if (owner.prev === undefined) parent.first = owner.next;
	else owner.prev.next = owner.next;
	if (owner.next === undefined) parent.last = owner.prev;
	else owner.next.prev = owner.prev;
	owner.prev = undefined;
	owner.next = undefined;
}

/** A scope-like owner on a detached Charm effect scope, whose cleanup ends it. Listed on `parent` when given. */
function scopeOwner(role: Role, parent: Owner | undefined, state: ObjectState | undefined): Owner {
	const owner: Owner = { role, node: undefined!, state, ended: false };
	owner.stop = effectScope(() => {
		owner.node = getActiveSub()!;
		return () => finish(owner);
	}, true);
	if (parent !== undefined) adopt(parent, owner);
	return owner;
}

/**
 * Idempotent. Latches the owner first, so a release that runs while it ends creates nothing under it. A Run is latched
 * by its body, because `ended` also marks the releases it runs before each rerun.
 */
function dispose(owner: Owner) {
	if (owner.role === Role.Run) {
		if (owner.body === undefined) return;
		owner.body = undefined;
	} else if (owner.ended) return;
	owner.ended = true;
	unlist(owner);
	if (owner.stop !== undefined) owner.stop();
	else if (owner.role === Role.Run) endRun(owner);
}

/**
 * Ends a Run that has no stop: a nested one, a queued one, or a root still in its first run. The Runs a nested one
 * created end first, then what it lists and holds. Closing it then has Charm drop its dependencies, at once or when
 * the batch or flush in progress ends.
 */
function endRun(run: Owner) {
	const node = run.node;
	if (run.close !== undefined && node !== undefined) {
		const children = new Array<Owner>();
		let link = node.deps;
		while (link !== undefined) {
			const child = nestedRuns.get(link.dep);
			if (child !== undefined) children.push(child);
			link = link.nextDep;
		}
		for (const child of children) dispose(child);
	}
	finish(run);
	run.close?.(true);
}

function generation(s: ObjectState): Owner {
	let owner = generations.get(s);
	if (owner === undefined) {
		const created = scopeOwner(Role.Generation, undefined, s);
		owner = created;
		generations.set(s, created);
		if (s.subscriptions === undefined) s.subscriptions = [];
		s.subscriptions.push(() => {
			generations.delete(s);
			dispose(created);
		});
	}
	return owner;
}

function interval(s: ObjectState): Owner {
	let owner = intervals.get(s);
	if (owner === undefined) {
		owner = scopeOwner(Role.Interval, undefined, s);
		intervals.set(s, owner);
	}
	return owner;
}

/**
 * @internal
 * The owner of something created now: the effect run, scope or release Crucible is running; else the running
 * Crucible object's enable interval while its Enable step runs, or its generation; else none. A returned owner may
 * have ended.
 */
export function resolveOwner(): Owner | undefined {
	const [s, context] = running();
	if (context !== undefined) return context as Owner;
	if (s === undefined) return undefined;
	if (hasEnded(s)) return { role: Role.Generation, node: undefined!, state: s, ended: true };
	return enabling() === s ? interval(s) : generation(s);
}

/** @internal Core edit E1: lifecycle calls it after `onDisable` and when it releases a generation. Idempotent. */
export function endInterval(s: ObjectState) {
	const owner = intervals.get(s);
	if (owner === undefined) return;
	intervals.delete(s);
	dispose(owner);
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

function reportRelease(owner: Owner, kind: Release, value: unknown, err: unknown) {
	runtimeDiagnostic(
		"error",
		`Crucible: cleanup ${releaseLabel(kind, value)} (${ownerLabel(owner.state)}) failed: ${tostring(err)}`,
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
function releaseEngine(owner: Owner, kind: Release, value: unknown) {
	let engine: (value: never) => void;
	if (kind === Release.Disconnect) engine = disconnect;
	else if (kind === Release.DestroyInstance) engine = destroyInstance;
	else engine = task.cancel;
	const [ok, err] = pcall(engine, value as never);
	if (!ok) pcall(reportRelease, owner, kind, value, err);
}

/** An owner's releases that run code, newest first, on one worker until one fails or yields. */
interface Batch {
	readonly owner: Owner;
	readonly releases: readonly unknown[];
	/** The index of the release running now. */
	at: number;
	/** That release's lifecycle scope is open. */
	scoped: boolean;
}

/**
 * Runs `batch` from `batch.at` down on the running worker. A release that runs code opens its own lifecycle scope, as
 * `within` would. One that leaves requests for a drain outside any other scope stops the batch with `true`, so the
 * caller drains before the next release, as that release's own `within` would have.
 */
function releaseBatch(batch: Batch): boolean {
	const owner = batch.owner;
	const releases = batch.releases;
	for (let i = batch.at; i >= 0; i -= 2) {
		batch.at = i;
		const kind = releases[i] as Release;
		const value = releases[i + 1];
		if (kind >= Release.Disconnect) {
			releaseEngine(owner, kind, value);
			continue;
		}
		batch.scoped = true;
		openScope();
		if (kind === Release.Call) (value as () => void)();
		else if (kind === Release.DestroyMethod) (value as Destroyable).destroy();
		else (value as UpperDestroyable).Destroy();
		batch.scoped = false;
		if (closeScope()) return true;
	}
	return false;
}

/** The release at `batch.at` threw or yielded, and its worker is gone: report it and close its scope. */
function failRelease(batch: Batch, value: unknown) {
	const at = batch.at;
	reportRelease(batch.owner, batch.releases[at] as Release, batch.releases[at + 1], value);
	if (!batch.scoped) return;
	batch.scoped = false;
	if (closeScope()) drain();
}

/**
 * Releases `releases` from index `from` down, under their ended owner. Engine values before the first function or
 * `destroy()` release here. From there one worker runs the rest as a batch. A release that throws or yields loses
 * that worker: it is cancelled and reported, and the batch goes on from the next release on a fresh one. Each
 * release's requests still apply when it returns, before the next one runs.
 */
function runReleases(owner: Owner, releases: readonly unknown[], from: number) {
	let batch: Batch | undefined;
	let i = from;
	while (i >= 0) {
		const kind = releases[i] as Release;
		if (kind >= Release.Disconnect) {
			releaseEngine(owner, kind, releases[i + 1]);
			i -= 2;
			continue;
		}
		if (batch === undefined) batch = { owner, releases, at: i, scoped: false };
		else batch.at = i;
		const [result, value] = call(releaseBatch, batch, undefined, owner.state, owner);
		if (result !== CallResult.Ok) failRelease(batch, value);
		else if (value === true) drain();
		i = batch.at - 2;
	}
}

/** Releases one value now, under its ended owner: a throw or a yield is reported and nobody is faulted. */
function releaseValue(owner: Owner, kind: Release, value: unknown) {
	if (kind >= Release.Disconnect) releaseEngine(owner, kind, value);
	else runReleases(owner, [kind, value], 0);
}

/** Arms a Run's cleanup on its Charm effect for the run now in progress, once. Charm clears it after it runs. */
function arm(run: Owner) {
	if (run.armed) return;
	run.armed = true;
	let release = run.finish;
	if (release === undefined) {
		release = () => finish(run);
		run.finish = release;
	}
	const active = setActiveSub(run.node);
	charmOnCleanup(release);
	setActiveSub(active);
}

function hold(owner: Owner, kind: Release, value: unknown) {
	let releases = owner.releases;
	if (releases === undefined) {
		releases = [];
		owner.releases = releases;
		if (owner.role === Role.Run) arm(owner);
	}
	const size = releases.size();
	releases[size] = kind;
	releases[size + 1] = value;
}

function register(owner: Owner | undefined, kind: Release, value: unknown) {
	if (owner === undefined) error(NO_OWNER, 0);
	if (isEnded(owner)) releaseValue(owner, kind, value);
	else hold(owner, kind, value);
}

/**
 * One run of a Crucible effect: its body on a worker, as its Run, in its own lifecycle scope, with what it returns
 * held as the first release. Lifecycle requests the body makes apply after it returns and its cleanup is held.
 */
function runEffect(run: Owner) {
	const body = run.body;
	if (body === undefined) return;
	run.node = getActiveSub()!;
	run.ended = false;
	openScope();
	const [result, value] = call(body, undefined, undefined, run.state, run);
	let failed = result !== CallResult.Ok;
	let failure = value;
	if (!failed && value !== undefined) {
		if (typeIs(value, "function")) register(run, Release.Call, value);
		else {
			failed = true;
			failure = `an effect returns nothing or a cleanup function; this one returned a ${typeOf(value)}`;
		}
	}
	if (closeScope()) drain();
	if (!failed) return;
	if (!run.creating) error(failure, 0);
	run.failure = failure;
}

/** A nested Run's Charm body: it reads its `closed` signal while it has a body, so closing it reruns it. */
function runNested(run: Owner) {
	if (run.body === undefined) return;
	if (run.node === undefined) nestedRuns.set(getActiveSub()!, run);
	(run.closed as Getter<boolean>)();
	runEffect(run);
}

/**
 * Creates `run`'s Charm effect, whose first run happens now: linked into the running effect when `lifetime` is a Run,
 * else a root that `lifetime` lists. A first run that fails ends the Run, its Charm effect included, and is rethrown.
 */
function create(run: Owner, lifetime: Owner) {
	const nested = lifetime.role === Role.Run;
	if (nested) {
		const [closed, close] = signal(false);
		run.closed = closed;
		run.close = close;
	}
	run.creating = true;
	const active = setActiveSub(nested ? lifetime.node : undefined);
	const [ok, stop] = pcall(charmEffect, nested ? () => runNested(run) : () => runEffect(run));
	setActiveSub(active);
	run.creating = undefined;
	if (ok && !nested) {
		run.stop = stop;
		// Disposed by its own first run, before Charm returned the stop.
		if (run.body === undefined) stop();
	}
	const failure = ok ? run.failure : stop;
	if (failure === undefined) return;
	run.failure = undefined;
	dispose(run);
	error(failure, 0);
}

/**
 * A new Run under `owner`, created now, or listed and created at boot's commit while boot is open. Inside a run it is
 * created at once, as the run's child.
 */
function startEffect(owner: Owner, body: () => unknown): () => void {
	const run: Owner = { role: Role.Run, node: undefined!, state: owner.state, ended: false, body };
	if (owner.role === Role.Run) create(run, owner);
	else {
		adopt(owner, run);
		if (pending !== undefined) pending.push(run);
		else create(run, owner);
	}
	return () => dispose(run);
}

/**
 * @internal
 * Boot opens: effects created from now on outside every run wait for `releaseEffects`. True when this call opened the
 * hold, false when one was already open.
 */
export function holdEffects(): boolean {
	if (pending !== undefined) return false;
	pending = [];
	return true;
}

/**
 * @internal
 * Boot closes. On commit, the waiting effects that are still live start in creation order inside one Charm batch, so
 * what their first runs write reaches other effects once. Each is checked as its turn comes, so one that an earlier
 * first run ended is skipped, and `create` stops one that its own first run disposed. A first run that throws is
 * reported with its owner and ends that effect, and the rest still start: a commit is never rolled back. On rollback
 * each one ends without running. Idempotent.
 */
export function releaseEffects(commit: boolean) {
	const waiting = pending;
	pending = undefined;
	if (waiting === undefined) return;
	if (!commit) {
		for (const run of waiting) dispose(run);
		return;
	}
	startBatch();
	for (const run of waiting) {
		// Ending an entry, or the owner that lists it, unlists it.
		const lifetime = run.parent;
		if (lifetime === undefined) continue;
		const [ok, err] = pcall(create, run, lifetime);
		if (!ok)
			runtimeDiagnostic(
				"error",
				`Crucible: an effect of ${ownerLabel(run.state)} failed at boot's commit: ${tostring(err)}`,
			);
	}
	settle("boot committed");
}

function noop() {}

/** @internal */
export function createEffect(body: () => unknown): () => void {
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	if (isEnded(owner)) return noop;
	return startEffect(owner, body);
}

/**
 * @internal
 * The baseline is read now, untracked, as the owner: its source runs as the owner on every run too, whoever wrote.
 */
export function createWatch<T>(source: () => T, onChange: (value: T, previous: T) => void): () => void {
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	const active = getActiveSub();
	const [result, baseline] = call(peek, source, undefined, owner.state, owner);
	// A source that throws or yields leaves its worker before `peek` restores the running effect's tracking.
	setActiveSub(active);
	if (result !== CallResult.Ok) error(baseline, 0);
	if (isEnded(owner)) return noop;
	let previous = baseline as T;
	return startEffect(owner, () => {
		const value = source();
		// A source that disposed its own watch ends it before onChange.
		if ((running()[1] as Owner).body === undefined || rawequal(value, previous)) return;
		const before = previous;
		previous = value;
		untracked(onChange, value, before);
	});
}

/** @internal Lifecycle requests the setup makes apply after it returns, and after a failed setup's releases. */
export function createScope(setup: () => void): () => void {
	const parent = resolveOwner();
	if (parent !== undefined && isEnded(parent)) return noop;
	const scope = scopeOwner(Role.Scope, parent, parent?.state);
	const active = setActiveSub(scope.node);
	openScope();
	const [result, value] = call(setup, undefined, undefined, scope.state, scope);
	setActiveSub(active);
	if (result !== CallResult.Ok) dispose(scope);
	if (closeScope()) drain();
	if (result !== CallResult.Ok) error(value, 0);
	return () => dispose(scope);
}

/** @internal */
export function registerCleanup(release: () => void) {
	const owner = resolveOwner();
	// A nil from untyped Luau would shift every pair after it, so it is dropped.
	if (release === undefined) return;
	register(owner, Release.Call, release);
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
	const owner = resolveOwner();
	register(owner, classify(value), value);
}
