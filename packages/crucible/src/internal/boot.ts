import { runtimeDiagnostic } from "./diagnostics";
import type { BootOptions } from "../index";
import type { CrucibleObject } from "../crucible-object";
import { Subsystem } from "../subsystem";
import { CrucibleBehaviour } from "../crucible-behaviour";
import { construct, forget, reconcile, useEngine, watch } from "./binding";
import { FrameHook, hooks, lifecycleHooks, shutdownHooks } from "./hooks";
import { useImplementerPort } from "./implementers";
import { admit, drain, hasEnded, request, track, useEffects } from "./lifecycle";
import { type EnginePort } from "./port";
import { records, seal, type BehaviourRecord, type ClassRecord, type SubsystemRecord } from "./registry";
import type { NetworkLifecycle } from "./network/session";
import { robloxPort, type BootPort } from "./roblox-port";
import { Scheduler } from "./scheduler";
import { Hook, Kind, Want, type ObjectState } from "./state";
import { holdBarrier, releaseBarrier, useReactivePort } from "./reactive/flush";
import { endInterval } from "./reactive/ownership";
import { rollbackDeclarations, sealDeclarations } from "./sync/declarations";

/** @internal */
export const BOOT_CLASSES = [Subsystem, CrucibleBehaviour];

/** @internal */
export interface Side {
	readonly name: "server" | "client";
	readonly renders: boolean;
	readonly booted: () => (() => void) | void;
	readonly network: NetworkLifecycle;
}

/** @internal */
export const PATTERN_PREFIX = "crucible-path:";
/** @internal */
export const SHUTDOWN_DEADLINE_SECONDS = 25;
/** @internal */
export const SHUTDOWN_TIMEOUT_SECONDS = 28;

let clock: (() => number) | undefined;
let sweep:
	| {
			readonly startTime: number;
			readonly order: readonly SubsystemRecord[];
			readonly started: ReadonlySet<SubsystemRecord>;
			readonly finished: ReadonlySet<SubsystemRecord>;
	  }
	| undefined;

/** @internal */
export interface ShutdownProgress {
	readonly startedAt: number;
	readonly forceStartAt: number;
	readonly deadline: number;
	/** Hooks in the sweep that have not finished, the caller's own excluded, counting those not yet started. */
	readonly pending: number;
	readonly notStarted: number;
}

/** @internal The one monotonic clock, core's and data's (data rules 25, 105). */
export function bootClock(): number {
	return (clock ?? os.clock)();
}

/** @internal Undefined until the sweep has begun. */
export function shutdownProgress(subsystem: Subsystem): ShutdownProgress | undefined {
	if (sweep === undefined) return undefined;
	let pending = 0;
	let notStarted = 0;
	for (const record of sweep.order) {
		if (sweep.finished.has(record) || record.state?.object === subsystem) continue;
		pending += 1;
		if (!sweep.started.has(record)) notStarted += 1;
	}
	return {
		startedAt: sweep.startTime,
		forceStartAt: sweep.startTime + SHUTDOWN_DEADLINE_SECONDS,
		deadline: sweep.startTime + SHUTDOWN_TIMEOUT_SECONDS,
		pending,
		notStarted,
	};
}

/** @internal */
export function parsePattern(pattern: string): readonly string[] {
	if (pattern.sub(1, PATTERN_PREFIX.size()) !== PATTERN_PREFIX)
		error(`Crucible load needs the transformer: ${pattern}`);
	const segments = pattern.sub(PATTERN_PREFIX.size() + 1).split("/");
	if (segments.some((s) => s === "" || s === "." || s === "..")) error(`Invalid Crucible load pattern: ${pattern}`);
	return segments;
}

function matches(name: string, pattern: string): boolean {
	const [open] = pattern.find("{", 1, true);
	if (open !== undefined) {
		const [close] = pattern.find("}", open, true);
		if (close === undefined) error(`Invalid Crucible name pattern: ${pattern}`);
		return pattern
			.sub(open + 1, close - 1)
			.split(",")
			.some((part) => matches(name, pattern.sub(1, open - 1) + part + pattern.sub(close + 1)));
	}
	const [star] = pattern.find("*", 1, true);
	if (star === undefined) return name === pattern;
	const prefix = pattern.sub(1, star - 1);
	if (name.sub(1, prefix.size()) !== prefix) return false;
	for (let i = prefix.size(); i <= name.size(); i++) if (matches(name.sub(i + 1), pattern.sub(star + 1))) return true;
	return false;
}

/** @internal */
export function load(patterns: readonly string[]) {
	const found = new Set<ModuleScript>();
	function walk(node: Instance, segments: readonly string[], index: number) {
		if (index === segments.size()) {
			if (node.IsA("ModuleScript")) found.add(node);
			return;
		}
		const segment = segments[index];
		if (segment === "**") walk(node, segments, index + 1);
		for (const child of node.GetChildren()) {
			if (segment === "**") walk(child, segments, index);
			else if (matches(child.Name, segment)) walk(child, segments, index + 1);
		}
	}
	for (const pattern of patterns) {
		const segments = parsePattern(pattern);
		walk(game.GetService(segments[0] as keyof Services), segments, 1);
	}
	const modules = [...found];
	modules.sort((a, b) => a.GetFullName() < b.GetFullName());
	for (const module of modules) require(module);
}

const HOOK_NAMES = [
	"constructor",
	"awake",
	"onEnable",
	"start",
	"onDisable",
	"onDestroy",
	"a frame hook",
	"accepts",
	"an All<T> callback",
	"settle",
	"cancel",
	"internal",
];
function where(port: EnginePort, s: ObjectState): string {
	const id = (s.record as ClassRecord).id;
	return s.instance === undefined ? id : `${id} on ${port.describe(s.instance)}`;
}
function describeFault(port: EnginePort, s: ObjectState): string {
	const fault = s.fault!;
	let message = `${where(port, s)} faulted in ${HOOK_NAMES[fault.hook]}: ${tostring(fault.error)}`;
	for (const later of fault.cleanup) message += `\n  then ${HOOK_NAMES[later.hook]} failed: ${tostring(later.error)}`;
	return message;
}
function hookCaller(s: ObjectState, hook: Hook): ((s: ObjectState) => void) | undefined {
	if (hook === Hook.Awake) return (s) => hooks().awake(s.object as CrucibleObject, 0);
	if (s.record.kind !== Kind.Behaviour) return undefined;
	if (hook === Hook.OnEnable) return (s) => lifecycleHooks().onEnable(s.object as CrucibleObject, 0);
	if (hook === Hook.OnDisable) return (s) => lifecycleHooks().onDisable(s.object as CrucibleObject, 0);
	if (hook === Hook.OnDestroy) return (s) => lifecycleHooks().onDestroy(s.object as CrucibleObject, 0);
	return undefined;
}

function shutdown(port: BootPort, order: readonly SubsystemRecord[]) {
	const started = new Set<SubsystemRecord>();
	const finished = new Set<SubsystemRecord>();
	const dependents = new Map<SubsystemRecord, SubsystemRecord[]>();
	for (const record of order) dependents.set(record, []);
	for (const record of order)
		for (const dep of record.resolved) if (!dep.all) dependents.get(dep.record)!.push(record);
	const startTime = port.clock();
	sweep = { startTime, order, started, finished };
	while (finished.size() < order.size()) {
		const expired = port.clock() - startTime >= SHUTDOWN_DEADLINE_SECONDS;
		for (const record of order) {
			if (started.has(record) || (!expired && !dependents.get(record)!.every((r) => finished.has(r)))) continue;
			started.add(record);
			port.spawn(() => {
				const object = record.state?.object;
				if (object !== undefined) {
					const [ok, err] = xpcall(
						() => shutdownHooks().onShutdown(object as Subsystem),
						(err) => debug.traceback(tostring(err), 2),
					);
					if (!ok) runtimeDiagnostic("error", `${record.id} onShutdown failed: ${err}`);
				}
				finished.add(record);
			});
		}
		if (port.clock() - startTime >= SHUTDOWN_TIMEOUT_SECONDS) return;
		if (finished.size() < order.size()) port.wait();
	}
}

/** @internal */
export function ignite(port: BootPort, side: Side, options: Pick<BootOptions, "fixedRate">) {
	clock = port.clock;
	sweep = undefined;
	const singletons = seal({ renders: side.renders });
	const frame = hooks().frame;
	const scheduler = new Scheduler(
		port,
		{ fixedRate: options.fixedRate ?? 60, renders: side.renders },
		{
			[FrameHook.PreRender]: (o, dt) => frame[FrameHook.PreRender](o as CrucibleObject, dt),
			[FrameHook.FixedUpdate]: (o, dt) => frame[FrameHook.FixedUpdate](o as CrucibleObject, dt),
			[FrameHook.Update]: (o, dt) => frame[FrameHook.Update](o as CrucibleObject, dt),
			[FrameHook.LateUpdate]: (o, dt) => frame[FrameHook.LateUpdate](o as CrucibleObject, dt),
		},
	);
	const owned = new Set<ObjectState>();
	let stage: "booting" | "rollback" | "committed" = "booting";
	let failure: string | undefined;
	let stop = () => {};
	function reject(message: string) {
		if (stage !== "booting") return;
		failure = message;
		stage = "rollback";
		stop();
		for (const state of owned) request(state, Want.Destroyed);
	}
	useEngine(port);
	holdBarrier();
	useReactivePort(port);
	useImplementerPort(port);
	useEffects({
		construct: (s) => {
			if (stage === "booting") owned.add(s);
			return construct(s);
		},
		hook: hookCaller,
		start: (s) => hooks().start(s.object as CrucibleObject, 0),
		join: (s) => {
			for (const set of (s.record as ClassRecord).memberOf) set.join(s.object!);
		},
		leave: (s) => {
			for (const set of (s.record as ClassRecord).memberOf) set.leave(s.object!);
		},
		tick: (s) => scheduler.add(s),
		untick: (s) => scheduler.remove(s),
		reconcile: () => {
			if (stage === "rollback") return [];
			const created = reconcile();
			if (stage === "booting") for (const s of created) owned.add(s);
			return created;
		},
		ended: (s) => {
			forget(s);
			if (s.fault === undefined) return;
			const message = describeFault(port, s);
			runtimeDiagnostic("error", message);
			if (s.record.kind === Kind.Subsystem) reject(message);
		},
		defer: port.defer,
		cancel: port.cancel,
		failed: (message) => {
			runtimeDiagnostic("error", message);
			reject(message);
		},
		diagnose: (s, hook, err) =>
			runtimeDiagnostic("error", `${where(port, s)} ${HOOK_NAMES[hook]} failed: ${tostring(err)}`),
		disabled: endInterval,
	});
	const [ok, err] = pcall(() => {
		stop = watch(records().filter((r): r is BehaviourRecord => r.kind === Kind.Behaviour));
		const initial = singletons.map((r) => {
			const s = track(r);
			r.state = s;
			owned.add(s);
			return s;
		});
		admit(initial);
		drain();
	});
	if (!ok) reject(tostring(err));
	if (failure === undefined) {
		let unbooted: (() => void) | void = undefined;
		const [finalized, finalizeErr] = pcall(() => {
			// Roblox cannot unbind BindToClose, so the callback checks that this boot committed.
			if (side.name === "server")
				port.bindToClose(() => {
					if (stage !== "committed") return;
					shutdown(port, singletons);
					side.network.close();
				});
			unbooted = side.booted();
			if (side.name === "client") sealDeclarations();
			side.network.commit();
		});
		if (!finalized) {
			const undo = unbooted as (() => void) | void;
			if (undo !== undefined) {
				const [undone, undoErr] = pcall(undo);
				if (!undone) runtimeDiagnostic("error", `Crucible: undoing booted failed: ${tostring(undoErr)}`);
			}
			reject(`boot finalization failed: ${tostring(finalizeErr)}`);
		}
	}
	if (failure !== undefined) {
		for (const s of owned) if (!hasEnded(s)) request(s, Want.Destroyed);
		drain();
		owned.clear();
		releaseBarrier();
		error(`Crucible boot failed: ${failure}`, 0);
	}
	stage = "committed";
	owned.clear();
	releaseBarrier();
}

/** @internal */
export function boot(side: Side, options: BootOptions) {
	const [ok, err] = pcall(() => {
		side.network.open();
		load(options.load);
		side.network.loaded();
		if (side.name === "server") sealDeclarations();
		ignite(robloxPort(), side, options);
	});
	if (ok) return;
	side.network.close();
	rollbackDeclarations();
	error(err, 0);
}
