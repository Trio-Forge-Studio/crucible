import { frame as checkFrame, validString } from "../network/codec";
import type { Branch, Literal, Node } from "../network/schema";
import {
	SYNC_ADMISSION_VISITS,
	SYNC_FRAME_BYTES,
	SYNC_FRAME_COLLECTION_ENTRIES,
	SYNC_FRAME_VALUE_VISITS,
	SYNC_STRING_BYTES,
	SYNC_VALUE_DEPTH,
} from "../reactive/limits";

/** @internal What the dynamic codec charges for a value: wire.md's bytes, entries and visits, and table nesting. */
export interface Cost {
	readonly bytes: number;
	readonly entries: number;
	readonly visits: number;
	readonly height: number;
}

/** @internal A peer examination's visit budget. A visit that would pass `limit` is not made. */
export interface Work {
	visits: number;
	readonly limit: number;
}

/** @internal Thrown when a visit would pass a `Work` limit: the peer is over budget, not the value. */
export interface OverBudget {
	readonly overBudget: true;
}

const OVER_BUDGET: OverBudget = table.freeze({ overBudget: true as const });

/** @internal */
export function isOverBudget(err: unknown): err is OverBudget {
	return rawequal(err, OVER_BUDGET);
}

/** @internal Charges one visit to `work`, checked before it is made. */
export function spend(work: Work) {
	if (work.visits >= work.limit) error(OVER_BUDGET);
	work.visits += 1;
}

type WeakSet = Map<object, true>;
function weak(): WeakSet {
	return setmetatable(new Map<object, true>(), { __mode: "k" });
}

/** @internal A descriptor compiled for admission: lookups by name and by discriminant, and a conformance cache. */
export type Plan =
	| { readonly kind: "nil" | "bool" | "string" | "vector3" | "color3" | "cframe" }
	| { readonly kind: "number"; readonly width: "f64" | "f32" | "u8" | "u16" | "u32" }
	| { readonly kind: "literal"; readonly value: Literal }
	| { readonly kind: "refuse"; readonly reason: string }
	| ObjectPlan
	| {
			readonly kind: "array";
			readonly element: Plan;
			readonly conforms: WeakSet;
	  }
	| { readonly kind: "tuple"; readonly elements: readonly Plan[]; readonly conforms: WeakSet }
	| { readonly kind: "map"; readonly keys?: ReadonlySet<string>; readonly value: Plan; readonly conforms: WeakSet }
	| UnionPlan;

interface ObjectPlan {
	readonly kind: "object";
	readonly fields: ReadonlyMap<string, Plan>;
	readonly order: readonly string[];
	readonly optional: ReadonlySet<string>;
	readonly conforms: WeakSet;
}

interface UnionPlan {
	readonly kind: "union";
	readonly nil?: Plan;
	readonly literals: ReadonlyMap<Literal, Plan>;
	readonly discriminant?: string;
	readonly variants: ReadonlyMap<Literal, Plan>;
	readonly kinds: ReadonlyMap<string, Plan>;
}

const RUNTIME_KINDS: { readonly [kind: string]: string } = {
	bool: "boolean",
	f64: "number",
	f32: "number",
	u8: "number",
	u16: "number",
	u32: "number",
	string: "string",
	vector3: "Vector3",
	color3: "Color3",
	cframe: "CFrame",
};

function admitsNil(plan: Plan): boolean {
	return plan.kind === "nil" || (plan.kind === "union" && plan.nil !== undefined);
}

/** @internal Compiles a descriptor the codec already accepted. Shared subtrees compile once. */
export function compilePlan(root: Node): Plan {
	const memo = new Map<Branch, Plan>();
	function object(fields: readonly (readonly [string, Node])[], extra?: readonly [string, Literal]): ObjectPlan {
		const byName = new Map<string, Plan>();
		const order = new Array<string>();
		const optional = new Set<string>();
		for (const [name, child] of fields) {
			const plan = build(child);
			byName.set(name, plan);
			order.push(name);
			if (admitsNil(plan)) optional.add(name);
		}
		if (extra !== undefined) {
			byName.set(extra[0], { kind: "literal", value: extra[1] });
			order.push(extra[0]);
		}
		return { kind: "object", fields: byName, order, optional, conforms: weak() };
	}
	function build(node: Branch): Plan {
		const known = memo.get(node);
		if (known !== undefined) return known;
		let plan: Plan;
		switch (node.kind) {
			case "nil":
			case "bool":
			case "string":
			case "vector3":
			case "color3":
			case "cframe":
				plan = { kind: node.kind };
				break;
			case "f64":
			case "f32":
			case "u8":
			case "u16":
			case "u32":
				plan = { kind: "number", width: node.kind };
				break;
			case "literal":
				plan = { kind: "literal", value: node.value };
				break;
			case "instance":
				plan = { kind: "refuse", reason: "replicated state cannot hold Instances; replicate a stable id" };
				break;
			case "dynamic":
				plan = { kind: "refuse", reason: "replicated state cannot hold an unknown value" };
				break;
			case "object":
				plan = object(node.fields);
				break;
			case "variant":
				plan = object(node.body.fields, [node.key, node.value]);
				break;
			case "array":
				plan = { kind: "array", element: build(node.element), conforms: weak() };
				break;
			case "tuple":
				plan = { kind: "tuple", elements: node.elements.map(build), conforms: weak() };
				break;
			case "map": {
				let keys: Set<string> | undefined;
				if (node.key.kind === "union") {
					keys = new Set();
					for (const branch of node.key.branches) keys.add(tostring(branch.value));
				}
				plan = { kind: "map", keys, value: build(node.value), conforms: weak() };
				break;
			}
			case "union": {
				const literals = new Map<Literal, Plan>();
				const variants = new Map<Literal, Plan>();
				const kinds = new Map<string, Plan>();
				let absent: Plan | undefined;
				let discriminant: string | undefined;
				for (const branch of node.branches) {
					const child = build(branch);
					if (branch.kind === "nil") absent = child;
					else if (branch.kind === "literal") literals.set(branch.value, child);
					else if (branch.kind === "variant") {
						discriminant = branch.key;
						variants.set(branch.value, child);
					} else kinds.set(RUNTIME_KINDS[branch.kind] ?? "table", child);
				}
				plan = { kind: "union", nil: absent, literals, discriminant, variants, kinds };
				break;
			}
		}
		memo.set(node, plan);
		return plan;
	}
	return build(root);
}

const costs = setmetatable(new Map<object, Cost>(), { __mode: "k" });

function varuintSize(value: number): number {
	let size = 1;
	while (value >= 128) {
		value = math.floor(value / 128);
		size += 1;
	}
	return size;
}

function stringBytes(text: string): number {
	return varuintSize(text.size()) + text.size();
}

/** @internal A leaf's dynamic cost, or an admitted table's cached cost. */
export function costOf(value: unknown): Cost {
	if (typeIs(value, "table")) return costs.get(value) ?? error("Crucible sync: costing a value admission never saw");
	let bytes = 1;
	if (typeIs(value, "number")) bytes = 9;
	else if (typeIs(value, "string")) bytes = 1 + stringBytes(value);
	else if (typeIs(value, "Vector3") || typeIs(value, "Color3")) bytes = 25;
	else if (typeIs(value, "CFrame")) bytes = 97;
	return { bytes, entries: 0, visits: 1, height: 0 };
}

function describe(value: unknown): string {
	return value === undefined ? "nil" : typeOf(value);
}

interface Pending {
	readonly plan: { readonly conforms: WeakSet };
	readonly value: object;
	readonly cost: Cost;
}

class Admission {
	steps = 0;
	bytes = 0;
	entries = 0;
	visits = 0;
	depth = 0;
	height = 0;
	readonly active = new Set<object>();
	readonly pending = new Array<Pending>();

	constructor(
		readonly name: string,
		readonly work: Work | undefined,
	) {}

	reject(path: string, reason: string): never {
		error(`${this.name}${path}: ${reason}`, 0);
	}

	/** One admission visit, checked before it is made: a value node, or a table key touched. */
	step(path: string) {
		if (this.work !== undefined) spend(this.work);
		if (this.steps >= SYNC_ADMISSION_VISITS) this.reject(path, "over SYNC_ADMISSION_VISITS");
		this.steps += 1;
	}

	charge(bytes: number, entries: number, visits: number, path: string) {
		this.bytes += bytes;
		this.entries += entries;
		this.visits += visits;
		if (this.bytes > SYNC_FRAME_BYTES) this.reject(path, "over SYNC_FRAME_BYTES");
		if (this.entries > SYNC_FRAME_COLLECTION_ENTRIES) this.reject(path, "over SYNC_FRAME_COLLECTION_ENTRIES");
		if (this.visits > SYNC_FRAME_VALUE_VISITS) this.reject(path, "over SYNC_FRAME_VALUE_VISITS");
	}

	chargeLeaf(value: unknown, path: string) {
		const cost = costOf(value);
		this.charge(cost.bytes, 0, 1, path);
	}

	/** Checks a table's shape, then returns its cached admission when it is frozen and known to conform. */
	open(plan: { readonly conforms: WeakSet }, value: unknown, path: string): object | undefined {
		if (!typeIs(value, "table")) this.reject(path, `expected table, got ${describe(value)}`);
		if (getmetatable(value) !== undefined) this.reject(path, "metatable");
		if (this.active.has(value)) this.reject(path, "cycle");
		if (table.isfrozen(value) && plan.conforms.has(value)) {
			const cost = costs.get(value)!;
			if (this.depth + cost.height > SYNC_VALUE_DEPTH) this.reject(path, "deeper than SYNC_VALUE_DEPTH");
			this.charge(cost.bytes, cost.entries, cost.visits, path);
			this.height = math.max(this.height, cost.height);
			return value;
		}
		return undefined;
	}

	/** Runs `walk` as one nested table: depth, cycle path, height and the subtree's cost. */
	nest(plan: { readonly conforms: WeakSet }, value: object, path: string, walk: () => object): object {
		this.depth += 1;
		if (this.depth > SYNC_VALUE_DEPTH) this.reject(path, "deeper than SYNC_VALUE_DEPTH");
		this.active.add(value);
		const outer = this.height;
		this.height = 0;
		const [bytes, entries, visits] = [this.bytes, this.entries, this.visits];
		const result = walk();
		const height = this.height + 1;
		this.pending.push({
			plan,
			value: result,
			cost: { bytes: this.bytes - bytes, entries: this.entries - entries, visits: this.visits - visits, height },
		});
		this.height = math.max(outer, height);
		this.active.delete(value);
		this.depth -= 1;
		return result;
	}
}

function admitNumber(a: Admission, width: string, value: unknown, path: string) {
	if (!typeIs(value, "number")) a.reject(path, `expected number, got ${describe(value)}`);
	if (value !== value || math.abs(value) === math.huge) a.reject(path, "expected finite number");
	if (width === "f32") {
		const scratch = buffer.create(4);
		buffer.writef32(scratch, 0, value);
		if (math.abs(buffer.readf32(scratch, 0)) === math.huge) a.reject(path, "f32 out of range");
	} else if (width !== "f64") {
		const maximum = width === "u8" ? 255 : width === "u16" ? 65535 : 4294967295;
		if (value < 0 || value > maximum || value % 1 !== 0) a.reject(path, `${width} out of range`);
	}
}

function admitString(a: Admission, value: unknown, path: string) {
	if (!typeIs(value, "string")) a.reject(path, `expected string, got ${describe(value)}`);
	if (value.size() > SYNC_STRING_BYTES) a.reject(path, "longer than SYNC_STRING_BYTES");
	const [ok] = pcall(validString, value);
	if (!ok) a.reject(path, "invalid UTF-8");
}

function admitNode(a: Admission, plan: Plan, value: unknown, path: string): unknown {
	a.step(path);
	switch (plan.kind) {
		case "nil":
			if (value !== undefined) a.reject(path, `expected nil, got ${describe(value)}`);
			break;
		case "bool":
			if (!typeIs(value, "boolean")) a.reject(path, `expected boolean, got ${describe(value)}`);
			break;
		case "number":
			admitNumber(a, plan.width, value, path);
			break;
		case "string":
			admitString(a, value, path);
			break;
		case "literal":
			if (value !== plan.value) a.reject(path, `expected a declared literal, got ${describe(value)}`);
			if (typeIs(value, "string")) admitString(a, value, path);
			break;
		case "vector3":
			if (!typeIs(value, "Vector3")) a.reject(path, `expected Vector3, got ${describe(value)}`);
			for (const component of [value.X, value.Y, value.Z])
				if (component !== component || math.abs(component) === math.huge)
					a.reject(path, "expected finite number");
			break;
		case "color3":
			if (!typeIs(value, "Color3")) a.reject(path, `expected Color3, got ${describe(value)}`);
			for (const component of [value.R, value.G, value.B])
				if (component !== component || math.abs(component) === math.huge)
					a.reject(path, "expected finite number");
			break;
		case "cframe": {
			if (!typeIs(value, "CFrame")) a.reject(path, `expected CFrame, got ${describe(value)}`);
			const [ok] = pcall(checkFrame, [...value.GetComponents()], path);
			if (!ok) a.reject(path, "invalid CFrame");
			break;
		}
		case "refuse":
			return a.reject(path, plan.reason);
		case "object":
			return admitObject(a, plan, value, path);
		case "array":
		case "tuple":
			return admitArray(a, plan, value, path);
		case "map":
			return admitMap(a, plan, value, path);
		case "union":
			return admitUnion(a, plan, value, path);
	}
	a.chargeLeaf(value, path);
	return value;
}

function admitField(a: Admission, name: string, plan: Plan, value: unknown, path: string): unknown {
	a.charge(stringBytes(name), 1, 1, path);
	return admitNode(a, plan, value, `${path}.${name}`);
}

function admitObject(a: Admission, plan: ObjectPlan, value: unknown, path: string): unknown {
	const cached = a.open(plan, value, path);
	if (cached !== undefined) return cached;
	const source = value as Record<string, unknown>;
	return a.nest(plan, source, path, () => {
		a.charge(1, 0, 1, path);
		const admitted = new Map<string, unknown>();
		let copy = false;
		let stopped = false;
		for (const [key, child] of pairs(source)) {
			a.step(path);
			const field = typeIs(key, "string") ? plan.fields.get(key) : undefined;
			if (field === undefined) {
				copy = true;
				stopped = true;
				break;
			}
			const result = admitField(a, key as string, field, child, path);
			if (!rawequal(result, child)) copy = true;
			admitted.set(key as string, result);
		}
		for (const name of plan.order) {
			if (admitted.has(name)) continue;
			let child: unknown;
			if (stopped) {
				a.step(path);
				child = source[name];
			}
			if (child === undefined) {
				if (!plan.optional.has(name)) a.reject(`${path}.${name}`, "missing");
				continue;
			}
			const result = admitField(a, name, plan.fields.get(name)!, child, path);
			if (!rawequal(result, child)) copy = true;
			admitted.set(name, result);
		}
		a.charge(varuintSize(admitted.size()), 0, 0, path);
		if (!copy) return source;
		const out: Record<string, unknown> = {};
		for (const [name, child] of admitted) out[name] = child;
		return out;
	});
}

function admitArray(
	a: Admission,
	plan: Extract<Plan, { kind: "array" | "tuple" }>,
	value: unknown,
	path: string,
): unknown {
	const cached = a.open(plan, value, path);
	if (cached !== undefined) return cached;
	const source = value as unknown[];
	return a.nest(plan, source, path, () => {
		const count = source.size();
		if (plan.kind === "tuple" && count !== plan.elements.size()) a.reject(path, "tuple length");
		a.charge(1 + varuintSize(count), count, 1, path);
		let seen = 0;
		for (const [key] of pairs(source)) {
			a.step(path);
			seen += 1;
			if (seen > count || !typeIs(key, "number") || key % 1 !== 0 || key < 1 || key > count)
				a.reject(path, "sparse array");
		}
		if (seen !== count) a.reject(path, "sparse array");
		let copy: unknown[] | undefined;
		for (let i = 0; i < count; i++) {
			const child = source[i];
			const result = admitNode(
				a,
				plan.kind === "array" ? plan.element : plan.elements[i],
				child,
				`${path}[${i + 1}]`,
			);
			if (!rawequal(result, child) && copy === undefined) copy = table.clone(source);
			if (copy !== undefined) copy[i] = result;
		}
		return copy ?? source;
	});
}

function admitMap(a: Admission, plan: Extract<Plan, { kind: "map" }>, value: unknown, path: string): unknown {
	const cached = a.open(plan, value, path);
	if (cached !== undefined) return cached;
	const source = value as Record<string, unknown>;
	return a.nest(plan, source, path, () => {
		a.charge(1, 0, 1, path);
		let count = 0;
		let copy: Record<string, unknown> | undefined;
		for (const [key, child] of pairs(source)) {
			a.step(path);
			if (!typeIs(key, "string") || (plan.keys !== undefined && !plan.keys.has(key)))
				a.reject(path, "invalid map key");
			count += 1;
			a.charge(stringBytes(key), 1, 1, path);
			const result = admitNode(a, plan.value, child, `${path}[]`);
			if (!rawequal(result, child) && copy === undefined) copy = table.clone(source);
			if (copy !== undefined) copy[key] = result;
		}
		a.charge(varuintSize(count), 0, 0, path);
		return copy ?? source;
	});
}

function admitUnion(a: Admission, plan: UnionPlan, value: unknown, path: string): unknown {
	if (value === undefined) {
		if (plan.nil === undefined) a.reject(path, "no branch for nil");
		return admitNode(a, plan.nil, value, path);
	}
	const literal = plan.literals.get(value as Literal);
	if (literal !== undefined) return admitNode(a, literal, value, path);
	const key = plan.discriminant;
	if (key !== undefined && typeIs(value, "table") && getmetatable(value) === undefined) {
		a.step(path);
		const variant = plan.variants.get((value as Record<string, unknown>)[key] as Literal);
		if (variant === undefined) a.reject(path, "no variant for its discriminant");
		return admitNode(a, variant, value, path);
	}
	const branch = plan.kinds.get(typeOf(value));
	if (branch === undefined) a.reject(path, `no branch for ${describe(value)}`);
	return admitNode(a, branch, value, path);
}

/**
 * @internal
 * Rule 49. Projects `value` through `plan`: undeclared keys are dropped unread, declared positions are checked, and
 * the result is frozen. Throws "<name><path>: <reason>" for a rejected value, never the value itself, and
 * `OverBudget` when a visit would pass `work`'s limit.
 */
export function admit(plan: Plan, name: string, value: unknown, work?: Work): unknown {
	const a = new Admission(name, work);
	const result = admitNode(a, plan, value, "");
	for (const entry of a.pending) {
		if (!table.isfrozen(entry.value)) table.freeze(entry.value);
		costs.set(entry.value, entry.cost);
		entry.plan.conforms.set(entry.value, true);
	}
	return result;
}

const BOUND = table.freeze({ bounded: true });

/**
 * @internal
 * Rule 61. Structural equality of two admitted values, free across `rawequal` subtrees. One visit per node pair and
 * per key iterated, charged to `work`; past SYNC_FRAME_VALUE_VISITS it answers not equal.
 */
export function equal(a: unknown, b: unknown, work?: Work): boolean {
	let spent = 0;
	function visit() {
		if (work !== undefined) spend(work);
		spent += 1;
		if (spent > SYNC_FRAME_VALUE_VISITS) error(BOUND);
	}
	function same(x: unknown, y: unknown): boolean {
		visit();
		if (rawequal(x, y)) return true;
		if (!typeIs(x, "table") || !typeIs(y, "table")) return typeOf(x) === typeOf(y) && x === y;
		const xs = x as unknown[];
		const ys = y as unknown[];
		if (xs.size() > 0 || ys.size() > 0) {
			if (xs.size() !== ys.size()) return false;
			for (let i = 0; i < ys.size(); i++) {
				visit();
				if (!same(xs[i], ys[i])) return false;
			}
			return true;
		}
		const xr = x as Record<string, unknown>;
		let count = 0;
		for (const [key, child] of pairs(y as Record<string, unknown>)) {
			visit();
			count += 1;
			const other = xr[key];
			if (other === undefined || !same(other, child)) return false;
		}
		let seen = 0;
		for (const _ of pairs(xr)) {
			visit();
			seen += 1;
			if (seen > count) return false;
		}
		return seen === count;
	}
	const [ok, result] = pcall(same, a, b);
	if (ok) return result;
	if (rawequal(result, BOUND)) return false;
	error(result, 0);
}

/**
 * @internal
 * Rule 63, client. The value to store for a `decoded` one: every subtree structurally equal to the matching subtree
 * of `current` is that subtree, so unchanged parts keep their identity. Returns `current` when the whole value is
 * equal. `decoded` is a fresh tree, so its new tables are frozen in place.
 */
export function reuse(current: unknown, decoded: unknown): unknown {
	if (!typeIs(decoded, "table"))
		return rawequal(current, decoded) || (typeOf(current) === typeOf(decoded) && current === decoded)
			? current
			: decoded;
	const old = typeIs(current, "table") ? (current as Record<string, unknown>) : undefined;
	const fresh = decoded as Record<string, unknown>;
	let same = old !== undefined;
	let count = 0;
	for (const [key, child] of pairs(fresh)) {
		count += 1;
		const before = old?.[key as string];
		const result = reuse(before, child);
		if (!rawequal(result, child)) fresh[key as string] = result;
		if (before === undefined || !rawequal(result, before)) same = false;
	}
	if (same) {
		let seen = 0;
		for (const _ of pairs(old!)) seen += 1;
		if (seen === count) return old;
	}
	return table.freeze(fresh);
}
