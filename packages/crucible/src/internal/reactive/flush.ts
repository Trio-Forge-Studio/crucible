import { runtimeDiagnostic } from "../diagnostics";
import type { EnginePort } from "../port";
import { CallResult } from "../runner";
import { FrameKind, pop, push, type Completion, type Frame } from "./context";
import {
	createdBefore,
	dependenciesMoved,
	guardPure,
	Mark,
	markEffect,
	NodeKind,
	observerFrame,
	originOf,
	readUntracked,
	rewire,
	staleAfterRun,
	useScheduler,
	nextId,
} from "./graph";
import { MAX_EFFECT_RUNS_PER_FLUSH, MAX_FLUSH_ROUNDS, MAX_RUNS_PER_EFFECT } from "./limits";
import {
	adopt,
	endLifetime,
	newLifetime,
	NO_OWNER,
	ownerLabel,
	resolveOwner,
	retire,
	Role,
	runBody,
	useDescribe,
	type EffectNode,
	type Lifetime,
} from "./ownership";

/** @internal What the flush asks of the engine. */
export type ReactivePort = Pick<EnginePort, "defer" | "describe">;

let port: ReactivePort | undefined;
/** Every realm starts held, so effects queued while modules load and boot runs wait for the commit. */
let held = true;
let requested = false;
let flushing = false;
let queue = new Array<EffectNode>();
/** The entries one round took from the queue, and the index of the first one not yet reached. */
let batch = new Array<EffectNode>();
let cursor = 0;
/** The entry being processed, which an internal error stops. */
let culprit: EffectNode | undefined;
/** Every effect whose run count moved this flush. */
let ran = new Array<EffectNode>();
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

function enqueue(effect: EffectNode) {
	if (effect.queued || effect.stopped) return;
	effect.queued = true;
	queue.push(effect);
	request();
}

useScheduler((effect) => enqueue(effect as EffectNode));

function report(level: "warning" | "error", message: string) {
	pcall(runtimeDiagnostic, level, message);
}

function fail(effect: EffectNode, reason: string) {
	retire(effect);
	report("error", `Crucible: effect at ${effect.origin} (${ownerLabel(effect.parent.owner)}) stopped: ${reason}`);
}

function createIn(owner: Lifetime, body: () => unknown) {
	const effect: EffectNode = {
		kind: NodeKind.Effect,
		id: nextId(),
		dependencies: [],
		mark: Mark.Dirty,
		stopped: false,
		queued: false,
		parent: owner,
		linked: false,
		body,
		origin: originOf(body),
		runs: 0,
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

/** @internal */
export function createWatch<T>(source: () => T, onChange: (value: T, previous: T) => void) {
	guardPure("create effects");
	const owner = resolveOwner();
	if (owner === undefined) error(NO_OWNER, 0);
	let previous = readUntracked(source);
	if (owner.ended) return;
	createIn(owner, () => {
		const value = source();
		if (rawequal(value, previous)) return;
		const before = previous;
		previous = value;
		readUntracked(() => onChange(value, before));
	});
}

interface Run {
	readonly effect: EffectNode;
	readonly frame: Frame;
}

function runEffectBody(run: Run) {
	push(run.frame);
	const result = run.effect.body();
	pop(run.frame);
	return result;
}

function complete(effect: EffectNode, run: Lifetime, frame: Frame, completion: Completion) {
	const value = completion.value;
	let failure: string | undefined;
	if (completion.result !== CallResult.Ok) failure = tostring(value);
	else if (value !== undefined && !typeIs(value, "function"))
		failure = `returned a ${typeOf(value)}; an effect returns nothing or a cleanup function`;
	if (failure !== undefined) {
		endLifetime(run);
		fail(effect, failure);
		return;
	}
	if (value !== undefined) run.releases.push(value as () => void);
	if (effect.stopped) return;
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
	const frame = observerFrame(FrameKind.Run, effect, run);
	runBody(run, runEffectBody, { effect, frame }, (completion) => complete(effect, run, frame, completion));
}

function carry() {
	const rest = new Array<EffectNode>();
	for (let i = cursor; i < batch.size(); i++) rest.push(batch[i]);
	for (const effect of queue) rest.push(effect);
	queue = rest;
	batch = [];
	cursor = 0;
}

function reportBound(bound: string) {
	if (boundReported) return;
	boundReported = true;
	const busiest = [...ran];
	busiest.sort((a, b) => a.runs > b.runs);
	const origins = new Array<string>();
	for (let i = 0; i < math.min(3, busiest.size()); i++) origins.push(`${busiest[i].origin} (${busiest[i].runs})`);
	report(
		"warning",
		`Crucible: the reactive flush reached ${bound}; the rest waits for the next flush. Most runs: ${origins.join(", ")}`,
	);
}

function runRounds() {
	let rounds = 0;
	let total = 0;
	while (queue.size() > 0) {
		if (rounds === MAX_FLUSH_ROUNDS) {
			reportBound(`${MAX_FLUSH_ROUNDS} rounds`);
			return;
		}
		rounds += 1;
		batch = queue;
		queue = [];
		cursor = 0;
		batch.sort(createdBefore);
		while (cursor < batch.size()) {
			const effect = batch[cursor];
			cursor += 1;
			culprit = effect;
			effect.queued = false;
			const mark = effect.mark;
			if (effect.stopped || mark === Mark.Clean) continue;
			effect.mark = Mark.Clean;
			if (mark === Mark.Check && !dependenciesMoved(effect)) continue;
			if (total === MAX_EFFECT_RUNS_PER_FLUSH) {
				effect.mark = Mark.Dirty;
				effect.queued = true;
				cursor -= 1;
				culprit = undefined;
				carry();
				reportBound(`${MAX_EFFECT_RUNS_PER_FLUSH} effect runs`);
				return;
			}
			if (effect.runs === MAX_RUNS_PER_EFFECT) {
				fail(effect, `ran ${MAX_RUNS_PER_EFFECT} times in one flush`);
				continue;
			}
			effect.runs += 1;
			total += 1;
			if (effect.runs === 1) ran.push(effect);
			runOnce(effect);
		}
		culprit = undefined;
		batch = [];
		cursor = 0;
	}
	boundReported = false;
}

function recover(err: unknown) {
	const blamed = culprit;
	carry();
	let stopFailure = "";
	if (blamed !== undefined) {
		const [ok, stopErr] = pcall(retire, blamed);
		if (!ok) {
			blamed.stopped = true;
			blamed.queued = false;
			stopFailure = `; stopping it failed: ${tostring(stopErr)}`;
		}
	}
	const where = blamed === undefined ? "" : ` while processing the effect at ${blamed.origin}`;
	report("error", `Crucible: internal error; the reactive flush recovered${where}: ${tostring(err)}${stopFailure}`);
}

/** @internal The deferred flush. Runs every queued effect to rest under the bounds. Never throws. */
export function flush() {
	requested = false;
	if (held || flushing) return;
	flushing = true;
	const [ok, err] = pcall(runRounds);
	if (!ok) recover(err);
	for (const effect of ran) effect.runs = 0;
	ran = [];
	batch = [];
	cursor = 0;
	culprit = undefined;
	flushing = false;
	if (queue.size() > 0) request();
}
