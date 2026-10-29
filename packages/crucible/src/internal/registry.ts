import type { CrucibleObject } from "../crucible-object";
import type { EmittedClass } from "../emit";
import { FrameHook, hooks, type HookMask } from "./hooks";
import { ImplementerSet } from "./implementers";
import { drain } from "./lifecycle";
import { Kind, Phase, type ObjectState } from "./state";
import type { SchedulerRecord } from "./scheduler";
import type { EmittedAttribute } from "../emit";

/** @internal */
export type TypeId = string & { readonly __crucibleTypeId: never };

/** @internal */
export interface Dep {
	readonly id: TypeId;
	readonly all: boolean;
}

/** @internal */
export type AttributeSchema = ReadonlyMap<string, EmittedAttribute>;

/** @internal */
export type AnyClass = new (...args: never[]) => CrucibleObject;

interface RecordBase extends SchedulerRecord {
	readonly id: TypeId;
	readonly klass: AnyClass;
	readonly hooks: HookMask;
	readonly deps: readonly Dep[];
	/** @internal */
	readonly provides: readonly TypeId[];
	/** @internal */
	readonly renderPriority?: number;
	/** @internal */
	resolved: readonly ResolvedDep[];
	/** @internal */
	memberOf: readonly ImplementerSet<object>[];
}

/** @internal */
export interface SubsystemRecord extends RecordBase {
	readonly kind: Kind.Subsystem;
	/** @internal */
	state?: ObjectState;
}

/** @internal */
export interface BehaviourRecord extends RecordBase {
	readonly kind: Kind.Behaviour;
	readonly tag: string;
	/** @internal */
	readonly instanceClasses: readonly string[];
	readonly attributes?: AttributeSchema;
	/** @internal */
	readonly bound: Map<Instance, ObjectState>;
	/** @internal */
	readonly suppressed: Set<Instance>;
}

/** @internal */
export type ClassRecord = SubsystemRecord | BehaviourRecord;

/** @internal */
export type ResolvedDep =
	| { readonly all: false; readonly record: SubsystemRecord }
	| { readonly all: true; readonly set: ImplementerSet<object> };

/** @internal */
export type SealProblem =
	| { readonly kind: "duplicate"; readonly id: TypeId }
	| { readonly kind: "missing"; readonly needer: TypeId; readonly dep: TypeId }
	| {
			readonly kind: "ambiguous";
			readonly needer: TypeId;
			readonly dep: TypeId;
			readonly providers: readonly TypeId[];
	  }
	| { readonly kind: "behaviour-dependency"; readonly needer: TypeId; readonly dep: TypeId }
	| { readonly kind: "cycle"; readonly path: readonly TypeId[] }
	| { readonly kind: "render-on-server"; readonly id: TypeId; readonly priority?: number };

/** @internal */
export interface SealPlan {
	readonly providers: ReadonlyMap<TypeId, readonly ClassRecord[]>;
	/** @internal */
	readonly subsystemOrder: readonly SubsystemRecord[];
}

/** @internal */
export type PlanResult =
	| { readonly ok: true; readonly plan: SealPlan }
	| { readonly ok: false; readonly problems: readonly SealProblem[] };

/** @internal */
export interface SealSide {
	readonly renders: boolean;
}

interface ClassStatics {
	readonly tag?: string;
	readonly renderPriority?: number;
}

const registered = new Array<ClassRecord>();
const byClass = new Map<object, ClassRecord>();
const duplicates = new Array<TypeId>();
const sets = new Map<TypeId, ImplementerSet<object>>();
let sealed: SealPlan | undefined;

/** @internal */
export function define(klass: AnyClass, emitted: EmittedClass) {
	if (sealed !== undefined)
		error(`Crucible: ${emitted.id} registered after boot; it would never be constructed or bound`);
	const id = emitted.id as TypeId;
	if (registered.some((record) => record.id === id)) duplicates.push(id);

	const statics = klass as unknown as ClassStatics;
	const base = {
		id,
		klass,
		hooks: hooks().overrides(klass),
		deps: emitted.deps.map((dep): Dep =>
			typeIs(dep, "string") ? { id: dep as TypeId, all: false } : { id: dep.all as TypeId, all: true },
		),
		provides: [id, ...emitted.provides.map((provided) => provided as TypeId)],
		renderPriority: statics.renderPriority,
		resolved: [],
		memberOf: [],
	};
	let record: ClassRecord;
	if (emitted.kind === "subsystem") {
		record = { ...base, kind: Kind.Subsystem };
	} else {
		const tag = emitted.tag;
		(klass as unknown as { tag: string }).tag = tag;
		let attributes: Map<string, EmittedAttribute> | undefined;
		if (emitted.attributes !== undefined) {
			attributes = new Map();
			for (const [name, check] of pairs(emitted.attributes)) attributes.set(name as string, check);
		}
		record = {
			...base,
			kind: Kind.Behaviour,
			tag,
			instanceClasses: emitted.instance,
			attributes,
			bound: new Map(),
			suppressed: new Set(),
		};
	}
	registered.push(record);
	byClass.set(klass, record);
}

/** @internal */
export function plan(records: readonly ClassRecord[], side: SealSide): PlanResult {
	const problems = new Array<SealProblem>();
	const providers = new Map<TypeId, ClassRecord[]>();
	for (const record of records) {
		for (const id of record.provides) {
			const list = providers.get(id);
			if (list === undefined) providers.set(id, [record]);
			else list.push(record);
		}
	}
	const single = new Map<ClassRecord, SubsystemRecord[]>();
	for (const record of records) {
		const edges = new Array<SubsystemRecord>();
		for (const dep of record.deps) {
			if (dep.all) continue;
			const candidates = providers.get(dep.id) ?? [];
			const subsystems = candidates.filter(
				(candidate): candidate is SubsystemRecord => candidate.kind === Kind.Subsystem,
			);
			if (candidates.size() === 0) problems.push({ kind: "missing", needer: record.id, dep: dep.id });
			else if (subsystems.size() === 0)
				problems.push({ kind: "behaviour-dependency", needer: record.id, dep: dep.id });
			else if (candidates.size() > 1)
				problems.push({
					kind: "ambiguous",
					needer: record.id,
					dep: dep.id,
					providers: candidates.map((c) => c.id),
				});
			else edges.push(subsystems[0]);
		}
		single.set(record, edges);
		const renders = (record.hooks & (1 << FrameHook.PreRender)) !== 0 || record.renderPriority !== undefined;
		if (!side.renders && renders)
			problems.push({ kind: "render-on-server", id: record.id, priority: record.renderPriority });
	}
	const order = new Array<SubsystemRecord>();
	const colour = new Map<SubsystemRecord, "grey" | "black">();
	const path = new Array<SubsystemRecord>();
	const visit = (record: SubsystemRecord) => {
		const seen = colour.get(record);
		if (seen === "black") return;
		if (seen === "grey") {
			const from = path.indexOf(record);
			const cycle = new Array<TypeId>();
			for (let i = from; i < path.size(); i++) cycle.push(path[i].id);
			cycle.push(record.id);
			problems.push({ kind: "cycle", path: cycle });
			return;
		}
		colour.set(record, "grey");
		path.push(record);
		for (const dep of single.get(record) ?? []) visit(dep);
		path.pop();
		colour.set(record, "black");
		order.push(record);
	};
	for (const record of records) if (record.kind === Kind.Subsystem) visit(record);
	if (problems.size() > 0) return { ok: false, problems };
	return { ok: true, plan: { providers, subsystemOrder: order } };
}

function describe(problem: SealProblem): string {
	switch (problem.kind) {
		case "duplicate":
			return `${problem.id} is registered twice`;
		case "missing":
			return `${problem.needer} needs ${problem.dep}, which nothing provides`;
		case "ambiguous":
			return `${problem.needer} needs one ${problem.dep}, but ${problem.providers.join(", ")} all provide it`;
		case "behaviour-dependency":
			return `${problem.needer} needs ${problem.dep}, which only behaviours provide; take All<T> or call X.get(instance)`;
		case "cycle":
			return `dependency cycle ${problem.path.join(" -> ")}; break it with All<T>`;
		case "render-on-server":
			return `${problem.id} renders but is loaded on the server`;
	}
}

/** @internal */
export function seal(side: SealSide): readonly SubsystemRecord[] {
	if (sealed !== undefined) error("Crucible: boot already ran on this side");
	const result = plan(registered, side);
	const problems = new Array<SealProblem>();
	for (const id of duplicates) problems.push({ kind: "duplicate", id });
	if (!result.ok) for (const problem of result.problems) problems.push(problem);
	if (problems.size() > 0) error(`Crucible could not boot:\n  ${problems.map(describe).join("\n  ")}`);
	if (!result.ok) return [];
	const { providers, subsystemOrder } = result.plan;
	for (const record of registered) {
		record.resolved = record.deps.map((dep): ResolvedDep => {
			if (dep.all) return { all: true, set: setFor(dep.id) };

			return { all: false, record: (providers.get(dep.id) ?? [])[0] as SubsystemRecord };
		});
		record.memberOf = record.provides.map(setFor);
	}
	sealed = result.plan;
	return subsystemOrder;
}

/** @internal */
export function records(): readonly ClassRecord[] {
	return registered;
}

/** @internal */
export function recordOf(klass: object): ClassRecord | undefined {
	return byClass.get(klass);
}

/** @internal */
export function resolveOne(id: TypeId): CrucibleObject {
	drain();
	const plan = sealed ?? error(`inject<${id}>() called before boot`);
	const candidates = plan.providers.get(id) ?? [];
	const provider = candidates[0];
	if (candidates.size() !== 1 || provider.kind !== Kind.Subsystem) {
		error(`inject<${id}>(): ${candidates.size()} providers; inject needs exactly one subsystem`);
	}
	const state = provider.state;
	if (state?.phase === Phase.Faulted) error(`inject<${id}>(): provider ${provider.id} is Faulted`);
	const object = state?.object;
	if (
		state === undefined ||
		object === undefined ||
		state.phase === Phase.Unconstructed ||
		state.phase === Phase.Constructed
	) {
		error(
			`inject<${id}>(): ${provider.id} is not awake yet; call inject from start or later, or take a constructor parameter`,
		);
	}
	return object as CrucibleObject;
}

/** @internal */
export function setFor(id: TypeId): ImplementerSet<object> {
	let set = sets.get(id);
	if (set === undefined) {
		set = new ImplementerSet(id);
		sets.set(id, set);
	}
	return set;
}
