import { runtimeDiagnostic } from "../diagnostics";
import type { EnginePort } from "../port";
import { CallResult } from "../runner";
import { FrameKind, type Completion } from "./context";
import {
	CacheKind,
	createComputedNode,
	createdBefore,
	dependenciesMoved,
	guardPure,
	link,
	Mark,
	markEffect,
	NodeKind,
	observerFrame,
	originOf,
	readUntracked,
	rewire,
	staleAfterRun,
	useScheduler,
	validate,
	nextId,
	type ComputedNode,
	type Reaction,
	type Source,
	type StateNode,
} from "./graph";
import { MAX_EFFECT_RUNS_PER_FLUSH, MAX_FLUSH_ROUNDS, MAX_RUNS_PER_EFFECT } from "./limits";
import {
	adopt,
	adoptBinding,
	endLifetime,
	hold,
	newLifetime,
	NO_OWNER,
	ownerLabel,
	Release,
	resolveOwner,
	retire,
	Role,
	runBody,
	useDescribe,
	type BindingNode,
	type EffectNode,
	type Lifetime,
	type ReactionNode,
} from "./ownership";

/** @internal What the flush asks of the engine. */
export type ReactivePort = Pick<EnginePort, "defer" | "describe">;

let port: ReactivePort | undefined;
/** Every realm starts held, so effects queued while modules load and boot runs wait for the commit. */
let held = true;
let requested = false;
let flushing = false;
let queue = new Array<ReactionNode>();
/** The entries one round took from the queue, and the index of the first one not yet reached. */
let batch = new Array<ReactionNode>();
let cursor = 0;
/** The entry being processed, which an internal error stops. */
let culprit: ReactionNode | undefined;
/** The binding whose sink is running. The flush's one `pcall` stops it alone when the sink throws. */
let sinking: BindingNode | undefined;
let rounds = 0;
let total = 0;
/** Runs this flush, for every effect or binding that ran. */
const runs = new Map<ReactionNode, number>();
/** A bound was reported, and no flush has finished within its bounds since. */
let boundReported = false;

/** @internal Boot installs the engine's port; until then an enqueue only queues. */
export function useReactivePort(value: ReactivePort | undefined) {
	port = value;
	useDescribe(value === undefined ? tostring : (instance) => value.describe(instance));
}

/** @internal Core edit E2. Idempotent. */
export function holdBarrier() {
	held = true;
}

/** @internal Core edit E2. Requests a flush when something is queued. */
export function releaseBarrier() {
	held = false;
	if (queue.size() > 0) request();
}

function request() {
	if (requested || flushing || held || port === undefined) return;
	requested = true;
	const [ok] = pcall(port.defer, flush);
	if (!ok) requested = false;
}

function enqueue(effect: ReactionNode) {
	queue.push(effect);
	request();
}

useScheduler(enqueue as (reaction: Reaction) => void);

function report(level: "warning" | "error", message: string) {
	pcall(runtimeDiagnostic, level, message);
}

function originOfReaction(reaction: ReactionNode): string {
	return reaction.kind === NodeKind.Effect ? reaction.origin : originOf(reaction.sink);
}

function fail(reaction: ReactionNode, reason: string) {
	retire(reaction);
	const what = reaction.kind === NodeKind.Effect ? "effect" : "binding";
	report(
		"error",
		`Crucible: ${what} at ${originOfReaction(reaction)} (${ownerLabel(reaction.parent?.owner)}) stopped: ${reason}`,
	);
}

function createIn(owner: Lifetime, body: () => unknown) {
	const effect: EffectNode = {
		kind: NodeKind.Effect,
		id: nextId(),
		dependencies: [],
		mark: Mark.Dirty,
		parent: owner,
		linked: false,
		body,
		origin: originOf(body),
	};
	adopt(owner, effect);
	enqueue(effect);
}

/** @internal */
export function createEffect(body: () => unknown) {
	guardPure("create effects");
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	if (owner.ended) return;
	createIn(owner, body);
}

function watchOwner(): Lifetime {
	guardPure("create effects");
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	return owner;
}

/** @internal */
export function createWatch<T>(source: () => T, onChange: (value: T, previous: T) => void) {
	const owner = watchOwner();
	const baseline = readUntracked(source);
	if (!owner.ended) watchFrom(owner, source, baseline, onChange);
}

/** @internal A watch whose baseline the caller already read untracked, so `source` is not read again for it. */
export function createWatchFrom<T>(source: () => T, baseline: T, onChange: (value: T, previous: T) => void) {
	const owner = watchOwner();
	if (!owner.ended) watchFrom(owner, source, baseline, onChange);
}

function watchFrom<T>(owner: Lifetime, source: () => T, baseline: T, onChange: (value: T, previous: T) => void) {
	let previous = baseline;
	createIn(owner, () => {
		const value = source();
		if (rawequal(value, previous)) return;
		const before = previous;
		previous = value;
		readUntracked(() => onChange(value, before));
	});
}

function bindIn<T>(
	parent: Lifetime | undefined,
	readable: () => T,
	baseline: T,
	sink: (target: never, value: T) => void,
	target: unknown,
): BindingNode {
	const read = readable as unknown as StateNode;
	const direct = typeIs(read, "table") && read.kind === NodeKind.State;
	const source: Source = direct ? read : createComputedNode(readable);
	const binding: BindingNode = {
		kind: NodeKind.Binding,
		id: nextId(),
		source,
		mark: direct ? Mark.Clean : Mark.Dirty,
		parent,
		sink: sink as (target: unknown, value: unknown) => void,
		target,
		value: baseline,
	};
	link(binding, source);
	if (parent !== undefined) adoptBinding(parent, binding);
	if (!direct) enqueue(binding);
	return binding;
}

/**
 * @internal
 * Binds `readable` under the current owner and returns the binding, or nothing when the owner has ended. On each flush
 * where its value is not `rawequal` to the last one, the flush calls `sink(target, value)` on its own thread, so
 * `sink` must not yield. Nothing runs at creation: the caller reads `baseline` untracked just before and applies it.
 * A state is read as it is. Any other readable is read through a computed, so a throw or a yield in it is cached as a
 * computed's would be, and the binding stops with a diagnostic as a watch does. So does a throw in `sink`.
 */
export function createBinding<T>(
	readable: () => T,
	baseline: T,
	sink: (target: never, value: T) => void,
	target: unknown,
): BindingNode | undefined {
	guardPure("create bindings");
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	if (owner.ended) return undefined;
	return bindIn(owner, readable, baseline, sink, target);
}

/** @internal `createBinding` with no owner: only `retire` ends it. */
export function createRootBinding<T>(
	readable: () => T,
	baseline: T,
	sink: (target: never, value: T) => void,
	target: unknown,
): BindingNode {
	guardPure("create bindings");
	return bindIn(undefined, readable, baseline, sink, target);
}

/** A Check binding's source is a computed. It moved when its cached value is no longer the one last passed. */
function bindingMoved(binding: BindingNode): boolean {
	const source = binding.source as ComputedNode;
	validate(source);
	const cache = source.cache;
	return cache.kind !== CacheKind.Value || !rawequal(cache.value, binding.value);
}

function runBinding(binding: BindingNode) {
	const source = binding.source;
	if (source.kind === NodeKind.Computed) validate(source);
	let value: unknown;
	if (source.kind === NodeKind.State) value = source.value;
	else {
		const cache = source.cache;
		if (cache.kind === CacheKind.Error) {
			fail(binding, cache.message);
			return;
		}
		value = cache.kind === CacheKind.Value ? cache.value : undefined;
	}
	if (rawequal(value, binding.value)) return;
	binding.value = value;
	sinking = binding;
	binding.sink(binding.target, value);
	sinking = undefined;
}

/** A run's frame names its effect and its run lifetime. */
function complete(completion: Completion) {
	const frame = completion.frame!;
	const effect = frame.observer as EffectNode;
	const run = frame.lifetime!;
	const value = completion.value;
	let failure: string | undefined;
	if (completion.result !== CallResult.Ok) failure = tostring(value);
	else if (value !== undefined && !typeIs(value, "function"))
		failure = `returned a ${typeOf(value)}; an effect returns nothing or a cleanup function`;
	if (failure !== undefined) {
		fail(effect, failure);
		return;
	}
	if (value !== undefined) hold(run, Release.Call, value);
	if (effect.mark === Mark.Stopped) return;
	const reads = frame.deps!;
	rewire(effect, effect.dependencies, reads);
	effect.dependencies = reads;
	markEffect(effect, staleAfterRun(effect));
}

function runOnce(effect: EffectNode) {
	const previous = effect.run;
	effect.run = undefined;
	if (previous !== undefined) endLifetime(previous);
	const run = newLifetime(Role.Run, effect.parent, effect.parent.owner, effect);
	effect.run = run;
	runBody(observerFrame(FrameKind.Run, effect, run), effect.body, undefined, complete);
}

function carry() {
	const rest = new Array<ReactionNode>();
	for (let i = cursor; i < batch.size(); i++) rest.push(batch[i]);
	for (const effect of queue) rest.push(effect);
	queue = rest;
	batch = [];
	cursor = 0;
}

function reportBound(bound: string) {
	if (boundReported) return;
	boundReported = true;
	const busiest = new Array<[reaction: ReactionNode, count: number]>();
	for (const entry of runs) busiest.push(entry);
	busiest.sort((a, b) => a[1] > b[1]);
	const origins = new Array<string>();
	for (let i = 0; i < math.min(3, busiest.size()); i++)
		origins.push(`${originOfReaction(busiest[i][0])} (${busiest[i][1]})`);
	report(
		"warning",
		`Crucible: the reactive flush reached ${bound}; the rest waits for the next flush. Most runs: ${origins.join(", ")}`,
	);
}

const sortIds = new Array<number>();
const sortNodes = new Map<number, ReactionNode>();

/** Ids are unique, so sorting them as numbers gives `createdBefore`'s order without a Luau call per comparison. */
function sortByCreation(list: ReactionNode[]) {
	for (let i = 0; i < list.size(); i++) {
		const node = list[i];
		sortIds[i] = node.id;
		sortNodes.set(node.id, node);
	}
	sortIds.sort();
	for (let i = 0; i < list.size(); i++) list[i] = sortNodes.get(sortIds[i])!;
	sortIds.clear();
	sortNodes.clear();
}

function inCreationOrder(list: readonly ReactionNode[]): boolean {
	for (let i = 1; i < list.size(); i++) if (createdBefore(list[i], list[i - 1])) return false;
	return true;
}

/** Goes on from `cursor`, so a run that a failed sink cut short resumes where it stopped. */
function runRounds() {
	for (;;) {
		while (cursor < batch.size()) {
			const effect = batch[cursor];
			cursor += 1;
			culprit = effect;
			const mark = effect.mark;
			if (mark === Mark.Stopped || mark === Mark.Clean) continue;
			effect.mark = Mark.Clean;
			if (mark === Mark.Check) {
				const moved = effect.kind === NodeKind.Binding ? bindingMoved(effect) : dependenciesMoved(effect);
				if (!moved) continue;
			}
			if (total === MAX_EFFECT_RUNS_PER_FLUSH) {
				// Validation can drain lifecycle work that stops this reaction.
				if ((effect.mark as Mark) !== Mark.Stopped) effect.mark = Mark.Dirty;
				cursor -= 1;
				culprit = undefined;
				carry();
				reportBound(`${MAX_EFFECT_RUNS_PER_FLUSH} effect runs`);
				return;
			}
			const count = runs.get(effect) ?? 0;
			if (count === MAX_RUNS_PER_EFFECT) {
				fail(effect, `ran ${MAX_RUNS_PER_EFFECT} times in one flush`);
				continue;
			}
			runs.set(effect, count + 1);
			total += 1;
			if (effect.kind === NodeKind.Binding) runBinding(effect);
			else runOnce(effect);
		}
		culprit = undefined;
		batch = [];
		cursor = 0;
		if (queue.isEmpty()) break;
		if (rounds === MAX_FLUSH_ROUNDS) {
			reportBound(`${MAX_FLUSH_ROUNDS} rounds`);
			return;
		}
		rounds += 1;
		batch = queue;
		queue = [];
		if (!inCreationOrder(batch)) sortByCreation(batch);
	}
	boundReported = false;
}

function resumeAfterSink(binding: BindingNode, err: unknown) {
	fail(binding, tostring(err));
	runRounds();
}

function recover(err: unknown) {
	const blamed = culprit;
	carry();
	let stopFailure = "";
	if (blamed !== undefined) {
		const [ok, stopErr] = pcall(retire, blamed);
		if (!ok) {
			blamed.mark = Mark.Stopped;
			stopFailure = `; stopping it failed: ${tostring(stopErr)}`;
		}
	}
	const where = blamed === undefined ? "" : ` while processing the effect at ${originOfReaction(blamed)}`;
	report("error", `Crucible: internal error; the reactive flush recovered${where}: ${tostring(err)}${stopFailure}`);
}

/** @internal The deferred flush. Runs every queued effect to rest under the bounds. Never throws. */
export function flush() {
	requested = false;
	if (held || flushing) return;
	flushing = true;
	rounds = 0;
	total = 0;
	let [ok, err] = pcall(runRounds);
	while (!ok && sinking !== undefined) {
		const binding = sinking;
		sinking = undefined;
		[ok, err] = pcall(resumeAfterSink, binding, err);
	}
	if (!ok) recover(err);
	runs.clear();
	batch = [];
	cursor = 0;
	culprit = undefined;
	flushing = false;
	if (queue.size() > 0) request();
}
