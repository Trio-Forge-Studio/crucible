import { patch } from "@rbxts/charm-sync";
import { encodeDynamic, Writer } from "../network/codec";
import { SYNC_FRAME_BYTES, SYNC_FRAME_COLLECTION_ENTRIES, SYNC_FRAME_VALUE_VISITS } from "../reactive/limits";
import { costOf, spend, type Work } from "./values";

// 0.4.1's declarations call this applyImmutable; its pinned Luau exports it as apply.
const applyPatch = (patch as unknown as { readonly apply: (current: unknown, delta: unknown) => unknown }).apply;

const encoded = setmetatable(new Map<object, object>(), { __mode: "k" });
const decoded = setmetatable(new Map<object, object>(), { __mode: "kv" });
const SAME = {};
const NIL = {};
const ARRAY_MARKER = "__array";
const patchSizes = setmetatable(new Map<object, number>(), { __mode: "k" });
let differences = new Map<unknown, Map<unknown, { readonly delta: unknown }>>();

// Charm's deletion marker is a plain table with __none = "__none". Escape that key, and the escape prefix,
// so a game's string-keyed map can contain either without becoming a deletion. String-key arrays too: Charm's
// fixArrays can mistake a sparse patch for a dense array when Luau's length operator reaches its last key.
function keys(value: unknown, cache: Map<object, object>, encode: boolean): unknown {
	if (!typeIs(value, "table")) return value;
	const known = cache.get(value);
	if (known !== undefined) return known === SAME ? value : known;
	const changes = new Array<readonly [string, string, unknown]>();
	const source = value as Record<string, unknown>;
	const array = encode ? rawget(value, 1) !== undefined : source[ARRAY_MARKER] === true;
	for (const [key, child] of pairs(source)) {
		if (!encode && array && key === ARRAY_MARKER) continue;
		let name = key;
		if (array) name = (encode ? tostring(key) : tonumber(key)) as string;
		else if (typeIs(key, "string")) {
			if (encode && (key === "__none" || key === ARRAY_MARKER || key.sub(1, 1) === "\0")) name = `\0${key}`;
			else if (!encode && key.sub(1, 1) === "\0") name = key.sub(2);
		}
		const updated = keys(child, cache, encode);
		if (name !== key || !rawequal(updated, child)) {
			changes.push([key, name, updated]);
		}
	}
	const result = changes.isEmpty() && !array ? source : table.clone(source);
	for (const [key, name] of changes) if (key !== name) result[key] = undefined;
	for (const [, name, updated] of changes) result[name] = updated;
	if (array) result[ARRAY_MARKER] = encode ? true : undefined;
	if (!table.isfrozen(result)) table.freeze(result);
	cache.set(value, result === value ? SAME : result);
	if (result !== value) {
		const reverse = encode ? decoded : encoded;
		reverse.set(result, value);
	}
	return result;
}

/** @internal A fresh cache per sync turn; shared public changes are diffed once across peers. */
export function beginPatches() {
	differences = new Map();
}

/** @internal Encode game keys without adding a table level to the value. */
export function snapshot(value: unknown): unknown {
	return keys(value, encoded, true);
}

/** @internal Charge both admitted input trees before entering Charm Sync's non-yielding diff. */
export function difference(current: unknown, updated: unknown, work: Work): unknown {
	const before = current === undefined ? NIL : current;
	const after = updated === undefined ? NIL : updated;
	let byNext = differences.get(before);
	if (byNext === undefined) {
		byNext = new Map();
		differences.set(before, byNext);
	}
	let found = byNext.get(after);
	if (found === undefined) {
		const visits = costOf(current).visits + costOf(updated).visits;
		if (visits > work.limit - work.visits) {
			work.visits = work.limit;
			spend(work);
		}
		work.visits += visits;
		found = { delta: patch.diff(snapshot(current), snapshot(updated)) };
		byNext.set(after, found);
	}
	return found.delta;
}

/** @internal Compare the actual patch encoding with the admitted full value; oversized patches are disposable. */
export function smaller(delta: unknown, value: unknown): boolean {
	// Charm Sync's diff returns a value that is not a table only as the new value itself, which encodes to its full cost.
	if (!typeIs(delta, "table")) return false;
	let bytes = patchSizes.get(delta);
	if (bytes === undefined) {
		const [ok, size] = pcall(() => {
			const writer = new Writer();
			encodeDynamic(writer, delta, "patch", new Set());
			if (writer.budget.entries > SYNC_FRAME_COLLECTION_ENTRIES || writer.budget.visits > SYNC_FRAME_VALUE_VISITS)
				return SYNC_FRAME_BYTES + 1;
			const [packet] = writer.finish(buffer.create(0));
			return buffer.len(packet);
		});
		bytes = ok ? size : SYNC_FRAME_BYTES + 1;
		patchSizes.set(delta, bytes);
	}
	return bytes < costOf(value).bytes;
}

/** @internal Apply one declaration without writing its atom; the caller commits the whole frame atomically. */
export function apply(current: unknown, delta: unknown, initial: boolean): unknown {
	if (initial) {
		const diff = patch.diff(snapshot(current), snapshot(delta));
		if (diff === undefined) return current;
		return keys(applyPatch(snapshot(current), diff), decoded, false);
	}
	const value = applyPatch(snapshot(current), delta);
	return keys(value, decoded, false);
}
