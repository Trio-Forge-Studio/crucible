import { call, CallResult } from "../runner";
import { admitsNil, n, runtimeKind, type Branch, type Node, type ObjectNode } from "../network/schema";
import { PERSIST_ADMISSION_VISITS, PERSIST_DEPTH, PERSIST_STRING_BYTES } from "./limits";

/** @internal A schema migration: a private copy of the stored record in, a patch out (rule 75). */
export type MigrationStep = (old: object) => object;

/** @internal What the durability engine knows of a profile or record schema. */
export interface Schema {
	readonly name: string;
	/** `migrations.size()` (rule 74). */
	readonly version: number;
	readonly defaults: object;
	/** The persist descriptor. Absent, a schema declares no field, so every key is undeclared and kept in place. */
	readonly node?: Node;
	readonly migrations?: readonly MigrationStep[];
}

/** @internal */
export type Loaded =
	| { readonly kind: "ok"; readonly value: object; readonly migrated: boolean }
	| { readonly kind: "unreadable"; readonly reason: string }
	| { readonly kind: "newer"; readonly version: number };

/** @internal */
export type Admission =
	| { readonly ok: true; readonly value: unknown }
	| { readonly ok: false; readonly path: string; readonly reason: string };

const OPEN: Node = n.object([]);
const FAILURE = {};
const JSON_NODE = {};
const F32_MAX = 3.4028234663852886e38;
const UINT_MAX = { u8: 255, u16: 65_535, u32: 4_294_967_295 };

interface Admitted {
	readonly node: object;
	readonly visits: number;
	readonly height: number;
}

/** A frozen table admitted under a node is not walked again (rule 15). */
const admitted = new WeakMap<object, Admitted>();

function fail(path: string, reason: string): never {
	throw [FAILURE, path, reason];
}

/** The length of a table whose keys are exactly 1..n, and undefined for any other table. */
function denseLength(value: object): number | undefined {
	const items = value as unknown[];
	const length = items.size();
	let count = 0;
	for (const [key] of pairs(value)) {
		if (!typeIs(key, "number") || key % 1 !== 0 || key < 1 || key > length) return undefined;
		count += 1;
	}
	return count === length ? length : undefined;
}

function hasOnlyStringKeys(value: object): boolean {
	for (const [key] of pairs(value)) if (!typeIs(key, "string")) return false;
	return true;
}

function describe(node: Branch): string {
	switch (node.kind) {
		case "bool":
			return "boolean";
		case "string":
			return "string";
		case "object":
		case "map":
			return "an object";
		case "array":
		case "tuple":
			return "an array";
		case "literal":
			return `the literal ${tostring(node.value)}`;
		case "variant":
			return `the variant ${node.key} = ${tostring(node.value)}`;
		case "union":
			return node.branches.map(describe).join(" or ");
		case "nil":
			return "nothing";
		default:
			return "number";
	}
}

function run(root: Node, rootValue: unknown, rootDefault: unknown): unknown {
	let visits = 0;
	/** Height of the subtree the last walk returned: 0 for a scalar, 1 plus its deepest child for a table. */
	let height = 0;
	const open = new Set<object>();

	function count(path: string) {
		visits += 1;
		if (visits > PERSIST_ADMISSION_VISITS) fail(path, `more than ${PERSIST_ADMISSION_VISITS} values`);
	}

	function enter(value: object, path: string, depth: number) {
		count(path);
		if (depth > PERSIST_DEPTH) fail(path, `deeper than ${PERSIST_DEPTH}`);
		if (getmetatable(value) !== undefined) fail(path, "a table with a metatable");
		if (open.has(value)) fail(path, "a cycle");
		open.add(value);
	}

	/** Reuses a frozen table already admitted under `node`, at its cached cost and with its depth checked. */
	function known(value: object, node: object, path: string, depth: number): boolean {
		const hit = admitted.get(value);
		if (hit === undefined || hit.node !== node || !table.isfrozen(value)) return false;
		visits += hit.visits;
		if (visits > PERSIST_ADMISSION_VISITS) fail(path, `more than ${PERSIST_ADMISSION_VISITS} values`);
		if (depth + hit.height - 1 > PERSIST_DEPTH) fail(path, `deeper than ${PERSIST_DEPTH}`);
		height = hit.height;
		return true;
	}

	function finish(source: object, copy: object, node: object, before: number, top: number, same: boolean): object {
		open.delete(source);
		const result = same && table.isfrozen(source) ? source : table.freeze(copy);
		height = top + 1;
		admitted.set(result, { node, visits: visits - before, height });
		return result;
	}

	function scalar(value: unknown, path: string): unknown {
		count(path);
		height = 0;
		return value;
	}

	function text(value: unknown, path: string): string {
		if (!typeIs(value, "string")) fail(path, `expected string, got ${typeOf(value)}`);
		if (value.size() > PERSIST_STRING_BYTES) fail(path, `a string over ${PERSIST_STRING_BYTES} bytes`);
		if (utf8.len(value)[0] === undefined) fail(path, "invalid UTF-8");
		return value;
	}

	function number(value: unknown, path: string): number {
		if (!typeIs(value, "number")) fail(path, `expected number, got ${typeOf(value)}`);
		if (value !== value || math.abs(value) === math.huge) fail(path, "expected a finite number");
		return value;
	}

	/** An undeclared value: storable JSON, copied and frozen, never interpreted (rule 76). */
	function json(value: unknown, path: string, depth: number): unknown {
		if (typeIs(value, "string")) return scalar(text(value, path), path);
		if (typeIs(value, "number")) return scalar(number(value, path), path);
		if (typeIs(value, "boolean")) return scalar(value, path);
		if (!typeIs(value, "table")) fail(path, `expected JSON, got ${typeOf(value)}`);
		if (known(value, JSON_NODE, path, depth)) return value;
		const before = visits;
		enter(value, path, depth);
		const length = denseLength(value);
		const copy: Record<string | number, unknown> = {};
		let top = 0;
		let same = true;
		if (length !== undefined && length > 0) {
			const items = value as unknown[];
			for (let i = 0; i < length; i++) {
				const child = json(items[i], `${path}[${i}]`, depth + 1);
				top = math.max(top, height);
				if (!rawequal(child, items[i])) same = false;
				copy[i + 1] = child;
			}
		} else {
			if (!hasOnlyStringKeys(value)) fail(path, "a table mixing array and object keys");
			for (const [key, entry] of pairs(value)) {
				const child = json(entry, `${path}.${key}`, depth + 1);
				top = math.max(top, height);
				if (!rawequal(child, entry)) same = false;
				copy[key as string] = child;
			}
		}
		return finish(value, copy, JSON_NODE, before, top, same);
	}

	function tableOf(value: unknown, path: string, expected: string): object {
		if (!typeIs(value, "table")) fail(path, `expected ${expected}, got ${typeOf(value)}`);
		return value;
	}

	function object(
		node: ObjectNode,
		value: unknown,
		fallback: unknown,
		path: string,
		depth: number,
		discriminant?: string,
	): unknown {
		const source = tableOf(value, path, "an object") as Record<string, unknown>;
		if (known(source, node, path, depth)) return source;
		const before = visits;
		enter(source, path, depth);
		if (!hasOnlyStringKeys(source)) fail(path, "expected an object");
		const defaults = typeIs(fallback, "table") ? (fallback as Record<string, unknown>) : undefined;
		const copy: Record<string, unknown> = {};
		const declared = new Set<string>();
		let top = 0;
		let same = true;
		if (discriminant !== undefined) {
			declared.add(discriminant);
			copy[discriminant] = source[discriminant];
			count(`${path}.${discriminant}`);
		}
		for (const [name, child] of node.fields) {
			declared.add(name);
			const field = source[name];
			const fieldPath = `${path}.${name}`;
			const fieldDefault = defaults?.[name];
			if (field === undefined) {
				if (admitsNil(child)) continue;
				if (fieldDefault === undefined)
					fail(fieldPath, "missing; a field added to existing data needs a migration step that fills it");
				copy[name] = fieldDefault;
				same = false;
				continue;
			}
			const result = walk(child, field, fieldDefault, fieldPath, depth + 1);
			top = math.max(top, height);
			if (!rawequal(result, field)) same = false;
			copy[name] = result;
		}
		for (const [key, entry] of pairs(source)) {
			if (declared.has(key as string)) continue;
			const result = json(entry, `${path}.${key}`, depth + 1);
			top = math.max(top, height);
			if (!rawequal(result, entry)) same = false;
			copy[key as string] = result;
		}
		return finish(source, copy, node, before, top, same);
	}

	function walk(node: Branch, value: unknown, fallback: unknown, path: string, depth: number): unknown {
		switch (node.kind) {
			case "nil":
				fail(path, "expected nothing");
			case "bool":
				if (!typeIs(value, "boolean")) fail(path, `expected boolean, got ${typeOf(value)}`);
				return scalar(value, path);
			case "string":
				return scalar(text(value, path), path);
			case "literal":
				if (value !== node.value) fail(path, `expected ${describe(node)}`);
				return scalar(value, path);
			case "f64":
				return scalar(number(value, path), path);
			case "f32":
				if (math.abs(number(value, path)) > F32_MAX) fail(path, "out of f32 range");
				return scalar(value, path);
			case "u8":
			case "u16":
			case "u32": {
				const amount = number(value, path);
				const maximum = UINT_MAX[node.kind];
				if (amount % 1 !== 0 || amount < 0 || amount > maximum)
					fail(path, `expected an integer in [0, ${maximum}]`);
				return scalar(amount, path);
			}
			case "object":
				return object(node, value, fallback, path, depth);
			case "array":
			case "tuple": {
				const source = tableOf(value, path, "an array");
				if (known(source, node, path, depth)) return source;
				const before = visits;
				enter(source, path, depth);
				const length = denseLength(source);
				if (length === undefined) fail(path, "expected an array");
				if (node.kind === "tuple" && length !== node.elements.size())
					fail(path, `expected ${node.elements.size()} elements`);
				const items = source as unknown[];
				const copy = new Array<defined>();
				let top = 0;
				let same = true;
				for (let i = 0; i < length; i++) {
					const element = node.kind === "tuple" ? node.elements[i] : node.element;
					const result = walk(element, items[i], undefined, `${path}[${i}]`, depth + 1);
					top = math.max(top, height);
					if (!rawequal(result, items[i])) same = false;
					copy.push(result as defined);
				}
				return finish(source, copy, node, before, top, same);
			}
			case "map": {
				const source = tableOf(value, path, "an object");
				if (known(source, node, path, depth)) return source;
				const before = visits;
				enter(source, path, depth);
				if (!hasOnlyStringKeys(source)) fail(path, "expected an object with string keys");
				const keys = new Set<string>();
				let restricted = false;
				if (node.key.kind === "union") {
					restricted = true;
					for (const branch of node.key.branches) keys.add(tostring(branch.value));
				} else if (node.key.kind !== "string") fail(path, "a saved map has string keys");
				const copy: Record<string, unknown> = {};
				let top = 0;
				let same = true;
				for (const [key, entry] of pairs(source)) {
					if (!typeIs(key, "string")) fail(path, "expected an object with string keys");
					if (restricted && !keys.has(key)) fail(`${path}.${key}`, "not a key this map declares");
					const result = walk(node.value, entry, undefined, `${path}.${key}`, depth + 1);
					top = math.max(top, height);
					if (!rawequal(result, entry)) same = false;
					copy[key] = result;
				}
				return finish(source, copy, node, before, top, same);
			}
			case "union": {
				const kind = typeOf(value);
				for (const branch of node.branches) {
					if (branch.kind === "nil") continue;
					if (branch.kind === "literal") {
						if (value === branch.value) return scalar(value, path);
					} else if (branch.kind === "variant") {
						if (!typeIs(value, "table") || (value as Record<string, unknown>)[branch.key] !== branch.value)
							continue;
						const own =
							typeIs(fallback, "table") &&
							(fallback as Record<string, unknown>)[branch.key] === branch.value;
						return object(branch.body, value, own ? fallback : undefined, path, depth, branch.key);
					} else if (runtimeKind(branch) === kind) return walk(branch, value, fallback, path, depth);
				}
				return fail(path, `expected ${describe(node)}, got ${kind}`);
			}
			default:
				return fail(path, `${node.kind} cannot be saved; a save holds JSON`);
		}
	}

	return walk(root, rootValue, rootDefault, "", 1);
}

/**
 * @internal Rules 14 and 76: validates declared positions, keeps undeclared keys in place, and returns a deeply frozen
 * copy. `defaults` fills a missing required key only along object fields (rule 76). Never throws on bad data, and a
 * failure names the path and the reason, never the value.
 */
export function admit(node: Node, value: unknown, defaults?: unknown): Admission {
	const [ok, result] = pcall(run, node, value, defaults);
	if (ok) return { ok: true, value: result };
	const [marker, path, reason] = result as [unknown, string, string];
	if (marker === FAILURE) return { ok: false, path, reason };
	throw result;
}

/** @internal The schema with its defaults admitted and frozen once (rule 4). */
export function sealSchema(schema: Schema): Schema {
	const node = schema.node ?? OPEN;
	const result = admit(node, schema.defaults);
	if (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);
	return { ...schema, node, defaults: result.value as object };
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeIs(value, "table") && hasOnlyStringKeys(value);
}

function copyJson(value: unknown): unknown {
	if (!typeIs(value, "table")) return value;
	const result: Record<string | number, unknown> = {};
	for (const [key, entry] of pairs(value)) result[key as string] = copyJson(entry);
	return result;
}

/** Rule 75: objects merge into the stored object, anything else replaces it verbatim. */
function merge(base: unknown, patch: unknown): unknown {
	if (!isObject(base) || !isObject(patch)) return patch;
	const result: Record<string, unknown> = { ...base };
	for (const [key, entry] of pairs(patch)) result[key] = merge(base[key], entry);
	return result;
}

/**
 * @internal Rules 74 to 77: a stored record at version v runs `migrations[v]` to the last step on a private copy, each
 * step merged as a patch, then the descriptor is applied (rule 76). An absent payload takes the defaults and runs no
 * migration (rule 18).
 */
export function loadRecord(schema: Schema, payload: object | undefined, version: number): Loaded {
	if (payload === undefined) return { kind: "ok", value: schema.defaults, migrated: false };
	if (version > schema.version) return { kind: "newer", version };
	let current: unknown = payload;
	let migrated = false;
	const steps = schema.migrations ?? [];
	for (let index = version; index < schema.version; index++) {
		const step = steps[index];
		if (step === undefined) continue;
		const [status, patch] = call(step, copyJson(current) as object, undefined);
		const label = `${schema.name}: migrations[${index}]`;
		if (status === CallResult.Threw) return { kind: "unreadable", reason: `${label} threw` };
		if (status === CallResult.Yielded) return { kind: "unreadable", reason: `${label} yielded` };
		if (!typeIs(patch, "table") || !isObject(patch))
			return { kind: "unreadable", reason: `${label} returned ${typeOf(patch)}, not a record` };
		current = merge(current, patch);
		migrated = true;
	}
	const result = admit(schema.node ?? OPEN, current, schema.defaults);
	if (!result.ok)
		return {
			kind: "unreadable",
			reason: `${schema.name}${result.path}: ${result.reason}${migrated ? ` (after migrating from version ${version})` : ""}`,
		};
	return { kind: "ok", value: result.value as object, migrated };
}

/** @internal Rule 14's walk over a callback's result, without defaults. Throws naming the store and path, never the value. */
export function admitWrite(schema: Schema, value: unknown): object {
	if (!typeIs(value, "table")) error(`${schema.name}: a store's value is a record, got ${typeOf(value)}`, 0);
	const result = admit(schema.node ?? OPEN, value);
	if (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);
	return result.value as object;
}
