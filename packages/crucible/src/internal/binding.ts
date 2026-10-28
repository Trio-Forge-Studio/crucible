import type { CrucibleObject } from "../crucible-object";
import { clearClaim, setClaim } from "./construction-claim";
import { drain, fault, hasEnded, isDraining, isLive, notePairs, request, track } from "./lifecycle";
import { LogLevel, type Disconnect, type EnginePort } from "./port";
import { recordOf, type BehaviourRecord, type ClassRecord } from "./registry";
import { call, CallResult } from "./runner";
import { Hook, Kind, Phase, Want, type ObjectState } from "./state";

interface Pair {
	readonly record: BehaviourRecord;
	readonly instance: Instance;
	connections?: Disconnect[];
	queued: boolean;
	parked: boolean;
	running: boolean;
	reset: boolean;
	wrongClass: boolean;
	readonly warnings: Map<string, string>;
}
interface AcceptsClass {
	accepts(instance: Instance): boolean;
}
let installed: EnginePort | undefined;
const pairsByRecord = new Map<BehaviourRecord, Map<Instance, Pair>>();
const wrongClasses = new Map<BehaviourRecord, Set<Instance>>();
const byTag = new Map<string, BehaviourRecord[]>();
let queued = new Array<Pair>();
let scheduled = false;
const getWarnings = new Set<string>();

function port(): EnginePort {
	return installed ?? error("Crucible binding used before boot");
}
/** @internal */
export function useEngine(engine: EnginePort) {
	installed = engine;
}
/** @internal */
export function defaultAccepts(instance: Instance): boolean {
	return port().bindingScope(instance);
}

/** @internal */
export function construct(state: ObjectState): CrucibleObject {
	const record = state.record as ClassRecord;
	const args = record.resolved.map((dep) => (dep.all ? dep.set : dep.record.state?.object));
	let worker: thread | undefined;
	const [result, value] = call(
		() => {
			worker = coroutine.running();
			setClaim(state);
			return new record.klass(...(args as unknown as never[]));
		},
		undefined,
		undefined,
		state,
	);
	// The constructor worker can be cancelled before its own cleanup runs.
	if (worker !== undefined) clearClaim(worker);
	if (result !== CallResult.Ok) error(value, 0);
	return value as CrucibleObject;
}

function pairFor(record: BehaviourRecord, instance: Instance): Pair {
	let instances = pairsByRecord.get(record);
	if (instances === undefined) {
		instances = new Map();
		pairsByRecord.set(record, instances);
	}
	let pair = instances.get(instance);
	if (pair === undefined) {
		pair = {
			record,
			instance,
			queued: false,
			parked: false,
			running: false,
			reset: false,
			wrongClass: wrongClasses.get(record)?.has(instance) ?? false,
			warnings: new Map(),
		};
		instances.set(instance, pair);
	}
	return pair;
}
function queue(pair: Pair) {
	if (pair.running) return;
	if (pair.parked) {
		pair.parked = false;
		pair.queued = false;
	}
	if (!pair.queued) {
		pair.queued = true;
		queued.push(pair);
	}
	notePairs();
}
function schedule() {
	if (scheduled) return;
	scheduled = true;
	port().defer(() => {
		scheduled = false;
		drain();
	});
}
function disconnect(pair: Pair) {
	const connections = pair.connections;
	pair.connections = undefined;
	if (connections !== undefined) for (const stop of connections) stop();
}
function warning(pair: Pair, field: string, condition: string, message: string) {
	if (pair.warnings.get(field) === condition) return;
	pair.warnings.set(field, condition);
	port().log(LogLevel.Warning, message);
}
function valueFor(pair: Pair, field: string, state?: ObjectState): LuaTuple<[boolean, unknown]> {
	const descriptor = pair.record.attributes?.get(field);
	if (descriptor === undefined) return $tuple(true, undefined);
	const value = port().getAttribute(pair.instance, descriptor.name);
	const valid = value === undefined ? !descriptor.required : descriptor.check(value);
	if (valid) {
		pair.warnings.delete(field);
		return $tuple(true, value);
	}
	const condition = typeOf(value);
	warning(
		pair,
		field,
		condition,
		`${pair.record.id} on ${port().describe(pair.instance)}: attribute ${descriptor.name} is invalid (${condition}); ${state === undefined ? "not bound" : "keeping the last valid value"}`,
	);
	return $tuple(false, value);
}
/** @internal */
export function applyAttribute(state: ObjectState, field: string) {
	const record = state.record as ClassRecord;
	if (record.kind !== Kind.Behaviour || state.instance === undefined || state.attributes === undefined) return;
	const pair = pairFor(record, state.instance);
	const [valid, value] = valueFor(pair, field, state);
	if (!valid) return;
	state.attributes[field] = value;
	if (state.object !== undefined) (state.object as Record<string, unknown>)[field] = value;
}
function connect(pair: Pair) {
	if (pair.connections !== undefined) return;
	const { record, instance } = pair;
	pair.connections = [
		port().onAncestryChanged(instance, () => {
			if (!port().inWorld(instance)) reset(pair);
			queue(pair);
			schedule();
		}),
	];
	for (const [field, descriptor] of record.attributes ?? new Map<string, import("../emit").EmittedAttribute>()) {
		pair.connections.push(
			port().onAttributeChanged(instance, descriptor.name, () => {
				const state = record.bound.get(instance);
				if (state !== undefined && !hasEnded(state)) applyAttribute(state, field);
				else {
					queue(pair);
					schedule();
				}
			}),
		);
	}
	const state = record.bound.get(instance);
	if (state !== undefined)
		for (const [field] of record.attributes ?? new Map<string, import("../emit").EmittedAttribute>())
			applyAttribute(state, field);
}
function reset(pair: Pair) {
	pair.reset = true;
	pair.record.suppressed.delete(pair.instance);
	pair.warnings.clear();
	disconnect(pair);
}
function callAccepts(klass: object, instance: Instance): boolean {
	return (klass as AcceptsClass).accepts(instance);
}
function accepted(pair: Pair, state?: ObjectState): boolean {
	const [result, value] = call(callAccepts, pair.record.klass, pair.instance, state);
	if (result === CallResult.Ok) return value === true;
	pair.record.suppressed.add(pair.instance);
	if (state !== undefined) fault(state, Hook.Accepts, value);
	else port().log(LogLevel.Error, `${pair.record.id}.accepts failed on ${port().describe(pair.instance)}: ${value}`);
	return false;
}
function evaluate(pair: Pair): ObjectState | undefined {
	const { record, instance } = pair;
	const engine = port();
	const present = engine.inWorld(instance) && engine.hasTag(instance, record.tag);
	if (!present) {
		reset(pair);
	}
	const have = record.bound.get(instance);
	if (have !== undefined) {
		if (present) connect(pair);
		if (pair.reset || !present || record.suppressed.has(instance)) {
			request(have, Want.Destroyed);
			return undefined;
		}
		if (
			!accepted(pair, have) ||
			pair.reset ||
			!engine.inWorld(instance) ||
			!engine.hasTag(instance, record.tag) ||
			record.suppressed.has(instance)
		)
			request(have, Want.Destroyed);
		return undefined;
	}
	if (pair.reset) {
		pair.reset = false;
		record.suppressed.delete(instance);
	}
	if (!present) {
		pairsByRecord.get(record)?.delete(instance);
		return undefined;
	}
	connect(pair);
	if (record.suppressed.has(instance)) return undefined;
	if (!record.instanceClasses.some((name) => engine.isA(instance, name))) {
		if (!pair.wrongClass) {
			pair.wrongClass = true;
			let warned = wrongClasses.get(record);
			if (warned === undefined) {
				warned = setmetatable(new Set<Instance>(), { __mode: "k" });
				wrongClasses.set(record, warned);
			}
			warned.add(instance);
			const className = record.id.split(":").pop() ?? record.id;
			engine.log(
				LogLevel.Warning,
				`${engine.describe(instance)} is a ${instance.ClassName}; ${className} needs a ${record.instanceClasses.join(" or ")}`,
			);
		}
		return undefined;
	}
	const attributes: Record<string, unknown> = {};
	let valid = true;
	for (const [field] of record.attributes ?? new Map<string, import("../emit").EmittedAttribute>()) {
		const [ok, value] = valueFor(pair, field);
		if (!ok) valid = false;
		attributes[field] = value;
	}
	if (!valid || !accepted(pair)) return undefined;
	if (
		pair.reset ||
		!engine.inWorld(instance) ||
		!engine.hasTag(instance, record.tag) ||
		record.suppressed.has(instance)
	)
		return undefined;
	const state = track(record, instance, attributes);
	record.bound.set(instance, state);
	return state;
}
/** @internal */
export function reconcile(): readonly ObjectState[] {
	const taken = queued;
	queued = [];
	const created = new Array<ObjectState>();
	for (const pair of taken) {
		pair.queued = false;
		pair.running = true;
		const state = evaluate(pair);
		pair.running = false;
		if (pair.reset && !pair.record.bound.has(pair.instance)) queue(pair);
		if (state !== undefined) created.push(state);
	}
	return created;
}
/** @internal */
export function watch(records: readonly BehaviourRecord[]): Disconnect {
	const connections = new Array<Disconnect>();
	for (const record of records) {
		const list = byTag.get(record.tag);
		if (list === undefined) byTag.set(record.tag, [record]);
		else list.push(record);
	}
	for (const [tag, records] of byTag) {
		connections.push(
			port().onTagAdded(tag, (instance) => {
				for (const record of records) queue(pairFor(record, instance));
				schedule();
			}),
		);
		connections.push(
			port().onTagRemoved(tag, (instance) => {
				for (const record of records) {
					const pair = pairFor(record, instance);
					reset(pair);
					queue(pair);
				}
				schedule();
			}),
		);
		for (const instance of port().tagged(tag)) for (const record of records) queue(pairFor(record, instance));
	}
	return () => {
		for (const stop of connections) stop();
		for (const [, instances] of pairsByRecord) for (const [, pair] of instances) disconnect(pair);
		byTag.clear();
	};
}
/** @internal */
export function refresh(instance: Instance): void {
	for (const [tag, records] of byTag)
		for (const record of records)
			if (port().hasTag(instance, tag) || record.bound.has(instance)) queue(pairFor(record, instance));
	drain();
}
/** @internal */
export function boundTo(klass: object, instance: Instance): ObjectState | undefined {
	const record = recordOf(klass);
	if (record === undefined || record.kind !== Kind.Behaviour) return undefined;
	const pair = pairFor(record, instance);
	queue(pair);
	if (isDraining()) {
		const state = record.bound.get(instance);
		if (
			state !== undefined
				? !isLive(state)
				: port().inWorld(instance) && port().hasTag(instance, record.tag) && !record.suppressed.has(instance)
		) {
			const [source, line] = debug.info(3, "sl");
			const site = `${source}:${line}`;
			if (!getWarnings.has(site)) {
				getWarnings.add(site);
				port().log(LogLevel.Warning, `Crucible get pending at ${site}`);
			}
		}
	} else {
		const held = queued.filter((other) => other !== pair);
		for (const other of held) other.parked = true;
		queued = [pair];
		drain();
		queued = [...held.filter((other) => other.parked), ...queued];
		for (const other of held) other.parked = false;
		if (queued.size() > 0) {
			notePairs();
			schedule();
		}
	}
	return record.bound.get(instance);
}
/** @internal */
export function remove(state: ObjectState): void {
	const record = state.record as ClassRecord;
	if (record.kind === Kind.Behaviour && state.instance !== undefined && !hasEnded(state))
		record.suppressed.add(state.instance);
	request(state, Want.Destroyed);
}
/** @internal */
export function forget(state: ObjectState): void {
	const record = state.record as ClassRecord;
	if (record.kind !== Kind.Behaviour || state.instance === undefined) return;
	if (record.bound.get(state.instance) === state) record.bound.delete(state.instance);
	const pair = pairFor(record, state.instance);
	if (state.phase === Phase.Faulted && !pair.reset) record.suppressed.add(state.instance);
	if (pair.reset) queue(pair);
}
