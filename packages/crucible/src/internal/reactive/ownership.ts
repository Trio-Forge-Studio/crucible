import { runtimeDiagnostic } from "../diagnostics";
import { current, enabling, hasEnded } from "../lifecycle";
import type { ClassRecord } from "../registry";
import { CallResult } from "../runner";
import type { ObjectState } from "../state";
import { boundary, FrameKind, pop, push, top, type Completion } from "./context";
import { contained, finishOpen, guardPure, NodeKind, originOf, releaseEdges, type EffectFields } from "./graph";
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
	readonly releases: (() => void)[];
	/** Latched first when it starts ending: nothing new attaches and no queued run of its effects starts. */
	ended: boolean;
	/** Its children and releases have been ended. */
	finished: boolean;
	/** Its own body, a run or a setup, is on the stack. */
	running: boolean;
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
		running: false,
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
	const frame = top();
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
	for (let i = releases.size() - 1; i >= 0; i--) runRelease(lifetime, releases[i]);
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

interface Release {
	readonly lifetime: Lifetime;
	readonly release: () => void;
}

function releaseBody(job: Release) {
	const frame = { kind: FrameKind.Owning, lifetime: job.lifetime };
	push(frame);
	job.release();
	pop(frame);
}

/**
 * @internal
 * Runs one release now, in its own boundary, as its ended lifetime: a yield is cancelled, a throw or yield is reported
 * and nobody is faulted. Inside a drain its lifecycle requests apply after the current step.
 */
export function runRelease(lifetime: Lifetime, release: () => void) {
	contained(lifetime.owner, releaseBody, { lifetime, release }, (completion) => {
		if (completion.result !== CallResult.Ok)
			runtimeDiagnostic(
				"error",
				`Crucible: cleanup at ${originOf(release)} (${ownerLabel(lifetime.owner)}) failed: ${tostring(completion.value)}`,
			);
	});
}

/**
 * @internal
 * Runs a lifetime's own body in a boundary. While it runs, endings that reach it wait; at completion `after` runs,
 * then those endings.
 */
export function runBody<A>(
	lifetime: Lifetime,
	body: (arg: A) => unknown,
	arg: A,
	after: (completion: Completion) => void,
) {
	lifetime.running = true;
	bodies.push(lifetime);
	boundary(lifetime.owner, body, arg, (completion) => {
		lifetime.running = false;
		const index = bodies.indexOf(lifetime);
		if (index >= 0) bodies.remove(index);
		const [ok, err] = pcall(() => {
			finishOpen(completion);
			after(completion);
		});
		const deferred = lifetime.deferred;
		lifetime.deferred = undefined;
		if (deferred !== undefined) for (const ending of deferred) finish(ending);
		if (!ok) error(err, 0);
	});
}

interface Setup {
	readonly scope: Lifetime;
	readonly setup: () => void;
}

function setupBody(job: Setup) {
	const frame = { kind: FrameKind.Owning, lifetime: job.scope };
	push(frame);
	job.setup();
	pop(frame);
}

/** @internal */
export function createScope(setup: () => void): () => void {
	guardPure("create scopes");
	const parent = resolveOwner();
	if (parent?.ended) return () => {};
	const scope = newLifetime(Role.Scope, parent, parent?.owner);
	if (parent !== undefined) adopt(parent, scope);
	let failed = false;
	let failure: unknown;
	runBody(scope, setupBody, { scope, setup }, (completion) => {
		if (completion.result === CallResult.Ok) return;
		failed = true;
		failure = completion.value;
		endLifetime(scope);
	});
	if (failed) error(failure, 0);
	return () => endLifetime(scope);
}

/** @internal */
export function registerCleanup(release: () => void) {
	guardPure("register cleanups");
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	if (owner.ended) runRelease(owner, release);
	else owner.releases.push(release);
}
