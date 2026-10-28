/** @internal Public API rule 1: a Subsystem has `awake` and `start` but no `onEnable`, `onDisable` or `onDestroy`. */
export const enum Kind {
	Subsystem,
	Behaviour,
}

/** @internal What lifecycle reads from a class record. The registry's records extend it. */
export interface LifeRecord {
	readonly kind: Kind;
}

/** @internal An object the `construct` effect built. Lifecycle never reads it. */
export type LifeObject = object;

/**
 * @internal
 * Where one generation of an object is. A phase also records which inverse hooks are owed: from Awake on,
 * one `onDestroy`; in Enabled, one `onDisable`. Each step enters its phase before running its hook, so a
 * hook that throws part way still owes its inverse. Faulted and Destroyed are terminal.
 *
 *   Unconstructed --constructor--> Constructed --awake--> Awake --onEnable--> Enabled <--onDisable / onEnable--> Disabled
 *         |                             |                   |  \--(want Disabled, no hook)---------------------------^
 *         +--------Drop, no hooks-------+                   +--------onDestroy (from Awake or Disabled)--> Destroyed | Faulted
 */
export const enum Phase {
	Unconstructed,
	Constructed,
	Awake,
	Enabled,
	Disabled,
	Faulted,
	Destroyed,
}

/** @internal `start` runs once per generation, the first time the object is Enabled with the gate at Start. */
export const enum StartPhase {
	Pending,
	Running,
	Done,
}

/**
 * @internal How far the object's batch has opened. A batch raises the gate on all its objects together, so
 * every object is awake before any is enabled and enabled before any starts. After its batch an object sits at Start.
 */
export const enum Gate {
	Closed,
	Construct,
	Awake,
	Enable,
	Start,
}

/** @internal What the outside world asked for. The latest request wins; Destroyed is absorbing. */
export const enum Want {
	Enabled,
	Disabled,
	Destroyed,
}

/** @internal Where a failure happened. The first one is the object's primary diagnostic. */
export const enum Hook {
	Construct,
	Awake,
	OnEnable,
	Start,
	OnDisable,
	OnDestroy,
	Frame,
	Accepts,
	Callback,
	Settle,
	/** Cancelling a suspended start thread failed. */
	Cancel,
	/** The reconciler or one of its effects failed while this generation's batch, step or release was unfinished. */
	Internal,
}

/** @internal One atomic move. Each runs at most one hook or changes one index. */
export const enum Step {
	/** Unconstructed -> Constructed. Runs the constructor. A throw leaves no object. */
	Construct,
	/** Constructed -> Awake. Runs `awake`. */
	Awake,
	/** Awake | Disabled -> Enabled. Runs `onEnable`. */
	Enable,
	/** Awake -> Disabled with no hook, for an object that wants to begin disabled. */
	BeginDisabled,
	/** Joins the `All<T>` sets after `onEnable` returned. */
	Join,
	/** Start Pending -> Running. Runs `start` on its own thread. */
	Start,
	/** Joins the frame lists after `start` returned. */
	Tick,
	/** Leaves the frame lists and the sets, before `onDisable`. */
	Leave,
	/** Enabled -> Disabled. Runs `onDisable`, closing the enable interval. */
	Disable,
	/** Awake | Disabled -> terminal. Stops a running start, runs `onDestroy`. */
	Destroy,
	/** Unconstructed | Constructed -> terminal with no hooks, because `awake` never ran. */
	Drop,
}

/** @internal Discriminates the two kinds of entry on the work stack. */
export const enum Work {
	Object,
	Batch,
}

/** @internal The first failure, with every later failure during its cleanup attached. */
export interface Fault {
	readonly hook: Hook;
	readonly error: unknown;
	readonly cleanup: { readonly hook: Hook; readonly error: unknown }[];
}

/** @internal The inputs `nextStep` reads. Lifecycle owns every field; nothing else writes them. */
export interface LifeState {
	phase: Phase;
	start: StartPhase;
	gate: Gate;
	want: Want;
	/** In every `All<T>` set the class provides. */
	joined: boolean;
	/** In the frame list of every frame hook the class overrides. */
	ticking: boolean;
	fault?: Fault;
}

/** @internal Runtime state of one generation. */
export interface ObjectState extends LifeState {
	readonly work: Work.Object;
	readonly record: LifeRecord;
	readonly instance?: Instance;
	/** A behaviour's validated attributes, read when binding admitted it. The behaviour holds it read-only. */
	readonly attributes?: Record<string, unknown>;
	/** Set by the Construct step. Undefined while Unconstructed and after a constructor throw. */
	object?: LifeObject;
	/** On the work stack. A request for a queued object only changes `want`; its next step reads it. */
	queued: boolean;
	/** Steps since a request from another causal root reached the object. `MAX_TRANSITIONS` faults it. */
	transitions: number;
	/** The outside event this generation's steps descend from. A step's requests carry its root. */
	root: number;
	startThread?: thread;
	/** Disconnects for the `All<T>.observe` subscriptions this object owns. Run when the generation ends. */
	subscriptions?: (() => void)[];
}

/** @internal */
export const enum Sweep {
	Construct,
	Awake,
	Enable,
	Start,
}

/** @internal New generations going through construct, awake, enable and start together. */
export interface Batch {
	readonly work: Work.Batch;
	readonly members: ObjectState[];
	/** The wave the construct and awake sweeps are walking. */
	wave: ObjectState[];
	/** Generations admitted during this batch's awake closure. They form the next wave. */
	pending: ObjectState[];
	sweep: Sweep;
	cursor: number;
	waves: number;
}

/**
 * @internal
 * Everything a step does outside lifecycle state. Boot installs the real ones; specs install doubles.
 * Lifecycle runs `construct` and every hook caller through `runner.call` with the object as owner, so a throw
 * or a yield comes back as a failure, and `start` on its own thread.
 */
export interface LifeEffects {
	/** Builds the object. May throw. */
	readonly construct: (s: ObjectState) => LifeObject;
	/** The caller for a non-yielding hook, or undefined when the class has none. */
	readonly hook: (s: ObjectState, hook: Hook) => ((s: ObjectState) => void) | undefined;
	/** Calls `start`. Runs on the generation's start thread and may yield. */
	readonly start: (s: ObjectState) => void;
	readonly join: (s: ObjectState) => void;
	readonly leave: (s: ObjectState) => void;
	readonly tick: (s: ObjectState) => void;
	readonly untick: (s: ObjectState) => void;
	/** Reconciles binding's queued (record, instance) pairs and returns the generations it created. */
	readonly reconcile: () => readonly ObjectState[];
	/** The generation ended: unbind, suppress the membership when Faulted, log the fault once. */
	readonly ended: (s: ObjectState) => void;
	readonly defer: (callback: () => void) => void;
	readonly cancel: (thread: thread) => void;
	/** A drain failure: a cap or an internal error. The drain already recovered; boot turns one into rejection. */
	readonly failed: (message: string) => void;
	/** A teardown failure on a generation with no fault. Logged; it does not fault the generation. */
	readonly diagnose: (s: ObjectState, hook: Hook, error: unknown) => void;
}
