import { call, CallResult, disown, own, ownerOf } from "./runner";
import type { Batch, LifeEffects, LifeObject, LifeRecord, LifeState, ObjectState } from "./state";
import { Gate, Hook, Kind, Phase, StartPhase, Step, Sweep, Want, Work } from "./state";

/**
 * @internal
 * Pure transition function. The drain calls it until it returns undefined. It reads only the current state,
 * so re-running it after a request converges on the same end state, and asking twice for the state an object
 * already has finds no step.
 */
export function nextStep(s: LifeState): Step | undefined {
	const ending = s.fault !== undefined || s.want === Want.Destroyed;
	switch (s.phase) {
		case Phase.Unconstructed:
			if (ending) return Step.Drop;
			return s.gate >= Gate.Construct ? Step.Construct : undefined;
		case Phase.Constructed:
			if (ending) return Step.Drop;
			return s.gate >= Gate.Awake ? Step.Awake : undefined;
		case Phase.Awake:
			if (ending) return Step.Destroy;
			if (s.gate < Gate.Enable) return undefined;
			return s.want === Want.Enabled ? Step.Enable : Step.BeginDisabled;
		case Phase.Enabled:
			if (s.fault !== undefined || s.want !== Want.Enabled)
				return s.joined || s.ticking ? Step.Leave : Step.Disable;
			if (!s.joined) return Step.Join;
			if (s.start === StartPhase.Pending) return s.gate >= Gate.Start ? Step.Start : undefined;
			return s.start === StartPhase.Done && !s.ticking ? Step.Tick : undefined;
		case Phase.Disabled:
			if (ending) return Step.Destroy;
			return s.want === Want.Enabled ? Step.Enable : undefined;
	}
	return undefined;
}

let installed: LifeEffects | undefined;
const states = new Map<LifeObject, ObjectState>();

/** @internal */
export function useEffects(effects: LifeEffects) {
	installed = effects;
}

function effects(): LifeEffects {
	return installed ?? error("Crucible lifecycle used before boot");
}

/** @internal */
export function stateOf(object: LifeObject): ObjectState | undefined {
	return states.get(object);
}

/** @internal Awake, not faulted, and not asked to go. `get` and queries see only live objects. */
export function isLive(s: LifeState): boolean {
	return (
		s.fault === undefined &&
		s.want !== Want.Destroyed &&
		(s.phase === Phase.Awake || s.phase === Phase.Enabled || s.phase === Phase.Disabled)
	);
}

/** @internal Faulted or Destroyed. */
export function hasEnded(s: LifeState): boolean {
	return s.phase === Phase.Faulted || s.phase === Phase.Destroyed;
}

/** @internal A new generation, before its constructor runs. `admit` gives it a batch. */
export function track(record: LifeRecord, instance?: Instance, attributes?: Record<string, unknown>): ObjectState {
	return {
		work: Work.Object,
		phase: Phase.Unconstructed,
		start: StartPhase.Pending,
		gate: Gate.Closed,
		want: Want.Enabled,
		joined: false,
		ticking: false,
		record,
		instance,
		attributes,
		queued: false,
		transitions: 0,
	};
}

/**
 * @internal
 * The object whose code is running: the owner of a runner worker during a hook or callback, or of a start
 * thread for its whole life. A thread made with `task.spawn` inside a hook has no owner.
 */
export function current(): ObjectState | undefined {
	return ownerOf(coroutine.running());
}

function note(s: ObjectState, hook: Hook, err: unknown) {
	if (s.fault === undefined) s.fault = { hook, error: err, cleanup: [] };
	else s.fault.cleanup.push({ hook, error: err });
}

/** A teardown failure attaches to the fault, or is diagnosed without faulting a clean generation. */
function cleanup(s: ObjectState, hook: Hook, err: unknown) {
	if (s.fault !== undefined) s.fault.cleanup.push({ hook, error: err });
	else effects().diagnose(s, hook, err);
}

const subsystemLacks: ReadonlySet<Hook> = new Set([Hook.OnEnable, Hook.OnDisable, Hook.OnDestroy]);

function run(s: ObjectState, hook: Hook) {
	if (s.record.kind === Kind.Subsystem && subsystemLacks.has(hook)) return;
	const caller = effects().hook(s, hook);
	if (caller === undefined) return;
	const [result, value] = call(caller, s, undefined, s);
	if (result !== CallResult.Ok) note(s, hook, value);
}

function construct(s: ObjectState): LifeObject {
	return effects().construct(s);
}

function release(s: ObjectState) {
	if (s.object !== undefined) states.delete(s.object);
	const subscriptions = s.subscriptions;
	s.subscriptions = undefined;
	if (subscriptions === undefined) return;
	for (const disconnect of subscriptions) {
		const [ok, err] = pcall(disconnect);
		if (!ok) cleanup(s, Hook.Internal, err);
	}
}

function finish(s: ObjectState) {
	s.phase = s.fault === undefined ? Phase.Destroyed : Phase.Faulted;
	release(s);
	effects().ended(s);
}

function traceback(err: unknown) {
	return debug.traceback(tostring(err), 2);
}

function runStart(s: ObjectState) {
	const [ok, err] = xpcall(effects().start, traceback, s);
	disown(coroutine.running());
	finishStart(s, ok, err);
}

function cancelSuspended(thread: thread): LuaTuple<[ok: boolean, err: unknown]> {
	disown(thread);
	return pcall(effects().cancel, thread);
}

/**
 * Only a suspended thread can be closed. A start thread that is running, or normal because it resumed the
 * thread now draining, keeps its owner, so `current()` sees an ended object and `observe` subscribes nothing.
 * A deferred check cancels it at its next yield, and `finishStart` ignores it if it returns first.
 */
function stopStart(s: ObjectState, thread: thread) {
	if (coroutine.status(thread) === "suspended") {
		const [ok, err] = cancelSuspended(thread);
		if (!ok) cleanup(s, Hook.Cancel, err);
		return;
	}
	const fx = effects();
	fx.defer(() => {
		if (coroutine.status(thread) !== "suspended") return;
		const [ok, err] = cancelSuspended(thread);
		if (!ok) fx.diagnose(s, Hook.Cancel, err);
	});
}

function perform(s: ObjectState, step: Step) {
	const fx = effects();
	switch (step) {
		case Step.Construct: {
			const [result, value] = call(construct, s, undefined, s);
			if (result !== CallResult.Ok) {
				note(s, Hook.Construct, value);
			} else {
				const object = value as LifeObject;
				s.object = object;
				states.set(object, s);
				s.phase = Phase.Constructed;
			}
			return;
		}
		case Step.Awake:
			s.phase = Phase.Awake;
			run(s, Hook.Awake);
			return;
		case Step.Enable:
			s.phase = Phase.Enabled;
			run(s, Hook.OnEnable);
			return;
		case Step.BeginDisabled:
			s.phase = Phase.Disabled;
			return;
		case Step.Join:
			s.joined = true;
			fx.join(s);
			return;
		case Step.Start: {
			s.start = StartPhase.Running;
			const thread = coroutine.create(runStart);
			s.startThread = thread;
			own(thread, s);
			const [ok, err] = coroutine.resume(thread, s);
			if (!ok) note(s, Hook.Start, err);
			return;
		}
		case Step.Tick:
			s.ticking = true;
			fx.tick(s);
			return;
		case Step.Leave:
			if (s.ticking) {
				s.ticking = false;
				fx.untick(s);
			}
			if (s.joined) {
				s.joined = false;
				fx.leave(s);
			}
			return;
		case Step.Disable:
			s.phase = Phase.Disabled;
			run(s, Hook.OnDisable);
			return;
		case Step.Destroy: {
			const thread = s.startThread;
			s.startThread = undefined;
			if (s.start === StartPhase.Running && thread !== undefined) stopStart(s, thread);
			run(s, Hook.OnDestroy);
			finish(s);
			return;
		}
		case Step.Drop:
			finish(s);
			return;
	}
}

/** Ends a generation with no user hooks, after its own teardown failed again. Every effect is protected. */
function forceEnd(s: ObjectState) {
	const fx = effects();
	const thread = s.startThread;
	s.startThread = undefined;
	if (s.start === StartPhase.Running && thread !== undefined) pcall(stopStart, s, thread);
	if (s.ticking) {
		s.ticking = false;
		pcall(fx.untick, s);
	}
	if (s.joined) {
		s.joined = false;
		pcall(fx.leave, s);
	}
	s.phase = s.fault === undefined ? Phase.Destroyed : Phase.Faulted;
	pcall(release, s);
	pcall(fx.ended, s);
}

/** @internal Steps one causal chain may take on one object before it faults. Fault cleanup runs past it. */
export const MAX_TRANSITIONS = 64;

/** @internal Creation that never settles is a failure after this many awake waves in one batch. */
export const MAX_AWAKE_WAVES = 1024;

/** @internal Batches one drain may start. A chain of objects creating objects in `start` stops here. */
export const MAX_BATCHES = 1024;

/**
 * @internal
 * Requests that steps may make in one drain. C5 bounds an object that undoes itself, not objects that undo each
 * other; this cap ends such a cycle. Like the batch cap it is a failure, and its value waits on a Studio measurement.
 */
export const MAX_CHAINED_REQUESTS = 65_536;

/** @internal Reconcile rounds one drain may run. Binding that queues pairs on every round stops here. */
export const MAX_RECONCILE_ROUNDS = 1024;

/** @internal Consecutive internal errors that fault or force-end nothing before the drain gives up on its work. */
export const MAX_RECOVERIES = 1;

type Entry = ObjectState | Batch;

function awaking(batch: Batch): boolean {
	return batch.sweep === Sweep.Construct || batch.sweep === Sweep.Awake;
}

function settle(reconciler: Reconciler) {
	reconciler.settle();
}

/**
 * The single reconciler. Every lifecycle and binding change happens in a drain, and one drain runs at a time.
 * A drain starts at an outermost entry point: an engine signal, boot, a frame dispatch, or a public call made
 * outside a drain. A request made inside a drain only enqueues, so no hook ever runs while another object's
 * hook is on the stack. The stack holds objects to settle and batches to advance. What one step asks for
 * collects in `fresh` and moves onto the stack when the step ends, first request on top. So requests run in
 * the order they were made, after the hook that made them returns and before its object takes its next step.
 */
class Reconciler {
	private readonly stack = new Array<Entry>();
	private readonly fresh = new Array<Entry>();
	/** Open scopes: the drain itself or a `within` callback. */
	private depth = 0;
	/** The generation whose step is running, which C5's chain length follows. */
	private stepping?: ObjectState;
	private pairsQueued = false;
	private rounds = 0;
	private batches = 0;
	private requests = 0;
	/** The first cap failure this drain. */
	private failure?: string;

	isDraining(): boolean {
		return this.depth > 0;
	}

	/** The only place `depth` changes. `call` never throws, so the decrement always runs. */
	private enter<A, B>(
		callback: (a: A, b: B) => unknown,
		a: A,
		b: B,
		owner?: ObjectState,
	): LuaTuple<[result: CallResult, value: unknown]> {
		this.depth += 1;
		const [result, value] = call(callback, a, b, owner);
		this.depth -= 1;
		return $tuple(result, value);
	}

	fail(message: string) {
		if (this.failure === undefined) this.failure = message;
	}

	notePairs() {
		this.pairsQueued = true;
	}

	push(s: ObjectState) {
		if (s.queued) return;
		s.queued = true;
		this.fresh.push(s);
	}

	/** C5: a request made from outside `s` restarts its count; one made by its own step does not. */
	chain(s: ObjectState) {
		const stepping = this.stepping;
		if (stepping !== s) s.transitions = 0;
		if (stepping === undefined) return;
		this.requests += 1;
		if (this.requests <= MAX_CHAINED_REQUESTS || stepping.fault !== undefined) return;
		// A cycle's members keep requesting while a passive target never does, so the requester is the one to end.
		this.requests = 0;
		const message = `Crucible: objects kept changing each other through ${MAX_CHAINED_REQUESTS} requests in one drain`;
		this.fail(message);
		note(stepping, Hook.Settle, message);
	}

	private raise(s: ObjectState, gate: Gate) {
		if (s.gate < gate) s.gate = gate;
		this.push(s);
	}

	private innermostBatch(): Batch | undefined {
		for (let i = this.fresh.size() - 1; i >= 0; i--) {
			const entry = this.fresh[i];
			if (entry.work === Work.Batch) return entry;
		}
		for (let i = this.stack.size() - 1; i >= 0; i--) {
			const entry = this.stack[i];
			if (entry.work === Work.Batch) return entry;
		}
		return undefined;
	}

	admit(created: readonly ObjectState[]) {
		if (created.size() === 0) return;
		const open = this.innermostBatch();
		if (open !== undefined && awaking(open)) {
			for (const s of created) open.pending.push(s);
			return;
		}
		if (this.batches === MAX_BATCHES) {
			for (const s of created) request(s, Want.Destroyed);
			this.fail(
				`Crucible: objects kept creating objects for ${MAX_BATCHES} batches in one drain; the rest were dropped`,
			);
			return;
		}
		this.batches += 1;
		this.fresh.push({
			work: Work.Batch,
			members: [],
			wave: [],
			pending: [...created],
			sweep: Sweep.Awake,
			cursor: 0,
			waves: 0,
		});
		this.drain();
	}

	abandon() {
		for (const list of [[...this.stack], [...this.fresh]]) {
			for (const entry of list) {
				if (entry.work !== Work.Batch) continue;
				for (const s of entry.members) request(s, Want.Destroyed);
				for (const s of entry.pending) request(s, Want.Destroyed);
			}
		}
	}

	/** Moves a batch one unit forward: raises one member's gate, or opens the next sweep. False when it is done. */
	private advance(batch: Batch): boolean {
		switch (batch.sweep) {
			case Sweep.Construct:
			case Sweep.Awake: {
				if (batch.cursor < batch.wave.size()) {
					const s = batch.wave[batch.cursor];
					batch.cursor += 1;
					this.raise(s, batch.sweep === Sweep.Construct ? Gate.Construct : Gate.Awake);
					return true;
				}
				batch.cursor = 0;
				if (batch.sweep === Sweep.Construct) {
					batch.sweep = Sweep.Awake;
					return true;
				}
				const wave = batch.pending;
				batch.pending = [];
				if (wave.size() === 0) {
					batch.sweep = Sweep.Enable;
				} else if (batch.waves === MAX_AWAKE_WAVES) {
					for (const s of wave) request(s, Want.Destroyed);
					this.fail(
						`Crucible: objects kept creating objects during awake for ${MAX_AWAKE_WAVES} waves; the last wave was dropped`,
					);
					batch.sweep = Sweep.Enable;
				} else {
					batch.waves += 1;
					batch.wave = wave;
					for (const s of wave) batch.members.push(s);
					batch.sweep = Sweep.Construct;
				}
				return true;
			}
			case Sweep.Enable:
			case Sweep.Start: {
				if (batch.cursor < batch.members.size()) {
					const s = batch.members[batch.cursor];
					batch.cursor += 1;
					this.raise(s, batch.sweep === Sweep.Enable ? Gate.Enable : Gate.Start);
					return true;
				}
				if (batch.sweep === Sweep.Start) return false;
				batch.cursor = 0;
				batch.sweep = Sweep.Start;
				return true;
			}
		}
		return false;
	}

	private settleStep(s: ObjectState): boolean {
		const step = nextStep(s);
		if (step === undefined) return false;
		s.transitions += 1;
		if (s.fault === undefined) {
			if (s.transitions > MAX_TRANSITIONS) {
				note(
					s,
					Hook.Settle,
					`did not settle after ${MAX_TRANSITIONS} transitions since a request from outside it`,
				);
				return true;
			}
		}
		this.stepping = s;
		perform(s, step);
		this.stepping = undefined;
		return true;
	}

	settle() {
		while (true) {
			while (this.fresh.size() > 0) this.stack.push(this.fresh.pop()!);
			const top: Entry | undefined = this.stack[this.stack.size() - 1];
			if (top === undefined || top.work === Work.Batch) {
				if (this.pairsQueued) {
					this.pairsQueued = false;
					if (this.rounds === MAX_RECONCILE_ROUNDS) {
						this.fail(
							`Crucible: binding kept queueing pairs for ${MAX_RECONCILE_ROUNDS} reconcile rounds in one drain; the rest wait for the next drain`,
						);
					} else {
						this.rounds += 1;
						this.admit(effects().reconcile());
					}
					continue;
				}
				if (top === undefined) return;
				if (!this.advance(top)) this.stack.pop();
				continue;
			}
			if (!this.settleStep(top)) {
				this.stack.pop();
				top.queued = false;
			}
		}
	}

	/**
	 * Empties the work after an internal error. Returns the generations the lost work would have moved on, in
	 * the order they would have run, with every unfinished batch's generations short of the start gate first:
	 * those batches are gone, so nothing else would ever raise their gates.
	 */
	private unwind(): LuaTuple<[stranded: ObjectState[], rest: ObjectState[]]> {
		const stranded = new Array<ObjectState>();
		const rest = new Array<ObjectState>();
		const order = [...this.fresh];
		for (let i = this.stack.size() - 1; i >= 0; i--) order.push(this.stack[i]);
		this.fresh.clear();
		this.stack.clear();
		for (const entry of order) {
			if (entry.work === Work.Object) {
				entry.queued = false;
				rest.push(entry);
				continue;
			}
			for (const s of entry.members) if (s.gate < Gate.Start) stranded.push(s);
			for (const s of entry.pending) if (s.gate < Gate.Start) stranded.push(s);
		}
		return $tuple(stranded, rest);
	}

	/**
	 * Runs every queued change to rest. Never throws: failures reach `effects.failed` once the drain recovered.
	 * After an internal error, the generation mid-step and every stranded batch generation fault with
	 * `Internal` and tear down through their normal steps. One whose teardown fails again is force-ended. Each
	 * retry faults or force-ends at least one generation, so recovery ends; when one would not, the drain
	 * force-ends what is left instead of retrying forever.
	 */
	drain() {
		if (this.depth > 0 || (this.stack.size() === 0 && this.fresh.size() === 0 && !this.pairsQueued)) return;
		const errors = new Array<string>();
		const faulted = new Set<ObjectState>();
		let idle = 0;
		let [result, err] = this.enter(settle, this, undefined);
		while (result !== CallResult.Ok) {
			errors.push(tostring(err));
			const culprit = this.stepping;
			this.stepping = undefined;
			const [stranded, rest] = this.unwind();
			let progressed = false;
			if (culprit !== undefined && !hasEnded(culprit)) {
				if (faulted.has(culprit)) forceEnd(culprit);
				else {
					faulted.add(culprit);
					note(culprit, Hook.Internal, err);
				}
				progressed = true;
			}
			for (const s of stranded) {
				if (hasEnded(s) || faulted.has(s)) continue;
				faulted.add(s);
				note(s, Hook.Internal, err);
				progressed = true;
			}
			idle = progressed ? 0 : idle + 1;
			const left = [...stranded, ...rest].filter((s) => !hasEnded(s));
			if (idle > MAX_RECOVERIES) {
				this.pairsQueued = false;
				for (const s of left) {
					if (hasEnded(s)) continue;
					if (!faulted.has(s)) note(s, Hook.Internal, err);
					forceEnd(s);
				}
				break;
			}
			for (const s of left) this.push(s);
			[result, err] = this.enter(settle, this, undefined);
		}
		this.rounds = 0;
		this.batches = 0;
		this.requests = 0;
		this.stepping = undefined;
		const failure = this.failure;
		this.failure = undefined;
		const fx = effects();
		for (const e of errors) fx.failed(`Crucible: internal error; the reconciler recovered: ${e}`);
		if (failure !== undefined) fx.failed(failure);
	}

	/** Runs `callback` as its own scope, so requests it makes apply after it returns. */
	within<A, B>(
		owner: ObjectState | undefined,
		blame: ObjectState | undefined,
		hook: Hook,
		callback: (a: A, b: B) => unknown,
		a: A,
		b: B,
	): LuaTuple<[result: CallResult, value: unknown]> {
		const outer = this.depth > 0;
		const [result, value] = this.enter(callback, a, b, owner);
		if (result !== CallResult.Ok && blame !== undefined) fault(blame, hook, value);
		if (!outer) this.drain();
		return $tuple(result, value);
	}
}

const reconciler = new Reconciler();

/** @internal True while a drain runs, including inside every hook and callback it calls. */
export function isDraining(): boolean {
	return reconciler.isDraining();
}

/**
 * @internal
 * Runs every queued change to rest. Returns at once when a drain is already running: that drain processes the
 * queue before it returns. Never throws. A cap or an internal error goes to `effects.failed` after recovery.
 */
export function drain() {
	reconciler.drain();
}

/**
 * @internal
 * Binding has queued pairs. The drain reconciles them when its top is a batch or it is empty. Past the round
 * cap the drain drops the note, so binding calls this again whenever it queues.
 */
export function notePairs() {
	reconciler.notePairs();
}

/**
 * @internal
 * Runs one callback as its own scope: a frame hook, an `All<T>` visit, or an observer callback. Requests it
 * makes apply after it returns. A failure faults `blame`; with no blame the caller logs the returned failure.
 * Outside a drain, the scope drains when it ends, so the next frame hook in a lane sees the effects of the one
 * before.
 */
export function within<A, B>(
	owner: ObjectState | undefined,
	blame: ObjectState | undefined,
	hook: Hook,
	callback: (a: A, b: B) => unknown,
	a: A,
	b: B,
): LuaTuple<[result: CallResult, value: unknown]> {
	return reconciler.within(owner, blame, hook, callback, a, b);
}

/** @internal Updates `want` and queues the object. Called by `setEnabled`, `remove()` and binding. Ignored once Destroyed is wanted. */
export function request(s: ObjectState, want: Want) {
	if (s.want === Want.Destroyed) return;
	s.want = want;
	reconciler.chain(s);
	reconciler.push(s);
	reconciler.drain();
}

/** @internal Faults the object from outside a lifecycle step: a frame hook, `accepts`, or an `All<T>` callback. */
export function fault(s: ObjectState, hook: Hook, err: unknown) {
	if (hasEnded(s)) return;
	note(s, hook, err);
	reconciler.chain(s);
	reconciler.push(s);
	reconciler.drain();
}

/** @internal Called by the start thread when `start` returns or throws. A start that outlived its generation is ignored. */
export function finishStart(s: ObjectState, ok: boolean, err: unknown) {
	if (hasEnded(s)) return;
	s.startThread = undefined;
	s.start = StartPhase.Done;
	if (!ok) note(s, Hook.Start, err);
	reconciler.chain(s);
	reconciler.push(s);
	reconciler.drain();
}

/**
 * @internal
 * Gives new generations a batch. Inside the awake closure of the innermost batch they join its next wave, so
 * every object a batch creates during awake is awake before any of the batch is enabled. Otherwise they form
 * a new batch, which runs to rest before the code that created them continues.
 */
export function admit(created: readonly ObjectState[]) {
	reconciler.admit(created);
}

/** @internal Asks every generation in an unfinished batch to end. Boot calls it when a Subsystem faults. */
export function abandon() {
	reconciler.abandon();
}
