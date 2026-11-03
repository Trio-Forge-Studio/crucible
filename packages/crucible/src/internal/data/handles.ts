import type { Node } from "../network/schema";
import { createComputedNode, readComputed, type ComputedNode } from "../reactive/graph";
import { childNode, describePath, getPath, luaKey, setPath, tableBranch, type Segment } from "./paths";

/**
 * @internal What a tree of handles reads from and writes to: a player's slot on the server, an inert source for a
 * player who is not present, or the owner's views on the client (rules 115, 117, 124, 126).
 */
export interface HandleSource {
	/** The store name, for messages. */
	readonly name: string;
	/** Client handles have no write members (rule 124). */
	readonly client: boolean;
	/** The body of a handle's read computed: a tracked read of the value at `path`. */
	read(path: readonly Segment[]): unknown;
	/** Throws where reading this source's handles is refused: its own session's claims (rule 123). */
	beforeRead(): void;
	/** One call of the write gate with a change built from the stored root (rule 12). */
	write(change: (root: object) => unknown): boolean;
}

interface HandleNode {
	readonly source: HandleSource;
	readonly path: readonly Segment[];
	/** The descriptor at this position, or undefined below a leaf or a union of several types. */
	readonly schema?: Node;
	/** Strong: a held handle keeps its ancestry, so reaching the path again finds the same handles (rule 121). */
	readonly parent?: object;
	/** Object-field children, strong: bounded by the template's shape. */
	readonly fields: Map<Segment, object>;
	/** Map-entry and array-element children, weak-valued: their keys are unbounded. */
	readonly entries: Map<Segment, object>;
	read?: ComputedNode;
}

/** The node lives in each handle's own metatable: Luau's weak keys are not ephemerons, so no shared table maps them. */
const NODE = {};

const RESERVED = new Set(["set", "update", "insert", "remove"]);

function nodeOf(target: unknown): HandleNode | undefined {
	if (!typeIs(target, "table")) return undefined;
	const meta = getmetatable(target) as Map<object, unknown> | undefined;
	return typeIs(meta, "table") ? (meta.get(NODE) as HandleNode | undefined) : undefined;
}

function mapKey(name: string): never {
	error(`a map key named "${name}" has no handle; read the map and write it with the map's update`, 3);
}

function writer(handle: unknown, method: string): HandleNode {
	const node = nodeOf(handle);
	if (node === undefined || node.source.client) mapKey(method);
	return node;
}

/** Rule 118: the parent of the written position exists and is a table, and an index is an integer in range. */
function check(node: HandleNode, root: object, path: readonly Segment[], extra: number): object {
	const name = node.source.name;
	let parent: unknown = root;
	for (let i = 0; i < path.size() - 1; i++) {
		parent = (parent as Record<Segment, unknown>)[luaKey(path[i])];
		if (!typeIs(parent, "table"))
			error(`${describePath(name, path)}: its parent does not exist; set the parent whole`, 0);
	}
	const last = path[path.size() - 1];
	if (typeIs(last, "number")) {
		const highest = (parent as unknown[]).size() - 1 + extra;
		if (last % 1 !== 0 || last < 0 || last > highest)
			error(
				`${describePath(name, path)}: ${
					highest < 0 ? "the array is empty" : `the index is not an integer from 0 to ${highest}`
				}`,
				0,
			);
	}
	return parent as object;
}

function method(name: string, run: (node: HandleNode, ...args: unknown[]) => boolean) {
	return setmetatable(
		{},
		{
			__call: (_, handle: unknown, ...args: unknown[]) => run(writer(handle, name), ...args),
			__index: () => mapKey(name),
			__newindex: () => mapKey(name),
		},
	);
}

/** The four write members, shared by every server handle (rules 118 to 120, 122). */
const METHODS = new Map<string, object>([
	[
		"set",
		method("set", (node, value) =>
			node.source.write((root) => {
				if (node.path.size() > 0) check(node, root, node.path, 0);
				return setPath(root, node.path, value);
			}),
		),
	],
	[
		"update",
		method("update", (node, change) =>
			node.source.write((root) => {
				if (node.path.size() > 0) check(node, root, node.path, 0);
				return setPath(root, node.path, (change as (current: unknown) => unknown)(getPath(root, node.path)));
			}),
		),
	],
	[
		"insert",
		method("insert", (node, value, index) =>
			node.source.write((root) => {
				const array = arrayAt(node, root);
				const position = index === undefined ? array.size() : (index as number);
				check(node, root, [...node.path, position], 1);
				const copy = table.clone(array) as defined[];
				copy.insert(position, value as defined);
				return setPath(root, node.path, copy);
			}),
		),
	],
	[
		"remove",
		method("remove", (node, index) =>
			node.source.write((root) => {
				const array = arrayAt(node, root);
				check(node, root, [...node.path, index as number], 0);
				const copy = table.clone(array) as defined[];
				copy.remove(index as number);
				return setPath(root, node.path, copy);
			}),
		),
	],
]);

function arrayAt(node: HandleNode, root: object): unknown[] {
	const value = getPath(root, node.path);
	if (!typeIs(value, "table"))
		error(`${describePath(node.source.name, node.path)}: it does not exist; set it whole`, 0);
	return value as unknown[];
}

function child(parent: object, node: HandleNode, key: Segment): object {
	const schema = childNode(node.schema, key);
	const strong = schema !== undefined && tableBranch(node.schema)?.kind === "object";
	const cache = strong ? node.fields : node.entries;
	const found = cache.get(key);
	if (found !== undefined) return found;
	const made = makeHandle({
		source: node.source,
		path: [...node.path, key],
		schema,
		parent,
		fields: new Map(),
		entries: setmetatable(new Map<Segment, object>(), { __mode: "v" }),
	});
	cache.set(key, made);
	return made;
}

const handleCall = (handle: object, ...args: unknown[]) => {
	const node = nodeOf(handle)!;
	const source = node.source;
	if (args.size() > 0)
		error(
			source.client
				? `${source.name}() is read-only on the client`
				: `${source.name}: read a handle with h(); write it with set or update`,
			2,
		);
	source.beforeRead();
	if (node.read === undefined) {
		const path = node.path;
		node.read = createComputedNode(() => source.read(path));
	}
	return readComputed(node.read);
};

const handleIndex = (handle: object, key: unknown) => {
	const node = nodeOf(handle)!;
	if (typeIs(key, "string") && RESERVED.has(key) && !node.source.client) return METHODS.get(key);
	if (!typeIs(key, "string") && !typeIs(key, "number")) error("a handle's children are reached by key or index", 2);
	return child(handle, node, key);
};

const handleNewindex = () => error("a handle has no fields to assign; write with set or update", 2);

/** @internal Rule 122: an empty table with a metatable of its own, holding its node under a private key. */
function makeHandle(node: HandleNode): object {
	const meta = new Map<unknown, unknown>([
		["__call", handleCall],
		["__index", handleIndex],
		["__newindex", handleNewindex],
		[NODE, node],
	]);
	return setmetatable({}, meta as LuaMetatable<object>);
}

/** @internal A root handle over `source`, typed by the descriptor `schema` (rules 115, 124). */
export function rootHandle(source: HandleSource, schema: Node): object {
	return makeHandle({
		source,
		path: [],
		schema,
		fields: new Map(),
		entries: setmetatable(new Map<Segment, object>(), { __mode: "v" }),
	});
}
