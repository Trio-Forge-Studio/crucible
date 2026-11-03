import type { Branch, Node } from "../network/schema";

/** @internal A path segment as TypeScript wrote it: a field or map key, or an array index from 0. */
export type Segment = string | number;

/** @internal Rule 116: TypeScript indices reach a handle as written; Luau arrays start at 1. */
export function luaKey(segment: Segment): Segment {
	return typeIs(segment, "number") ? segment + 1 : segment;
}

/** @internal The value at `path`, or undefined where a table on the way is absent. Never throws. */
export function getPath(root: unknown, path: readonly Segment[], from = 0): unknown {
	let current = root;
	for (let i = from; i < path.size(); i++) {
		if (!typeIs(current, "table")) return undefined;
		current = (current as Record<Segment, unknown>)[luaKey(path[i])];
	}
	return current;
}

/**
 * @internal Rule 118's path copy: each table on the path is `table.clone` of the stored one with one field replaced,
 * which is what a spread compiles to, so undeclared keys stay in place at every level (rule 78). A leaf rawequal to the
 * stored one returns `root` itself, which is no revision (rule 12). Admission freezes what this builds (rule 15).
 */
export function setPath(root: unknown, path: readonly Segment[], leaf: unknown, from = 0): unknown {
	if (from >= path.size()) return leaf;
	const current = root as Record<Segment, unknown>;
	const key = luaKey(path[from]);
	const child = current[key];
	const replaced = setPath(child, path, leaf, from + 1);
	if (rawequal(replaced, child)) return root;
	const copy = table.clone(current);
	copy[key] = replaced;
	return copy;
}

/** @internal `FD.Players.career.jobs[0].coins`: keys after dots, indices from 0 in brackets. */
export function describePath(name: string, path: readonly Segment[], upTo = path.size()): string {
	let text = name;
	for (let i = 0; i < upTo; i++) {
		const segment = path[i];
		text += typeIs(segment, "number") ? `[${segment}]` : `.${segment}`;
	}
	return text;
}

/** @internal The one table branch of an optional table (`V | undefined`), which keeps its children (rule 116). */
export function tableBranch(node: Node | undefined): Node | undefined {
	if (node === undefined || node.kind !== "union") return node;
	let found: Branch | undefined;
	for (const branch of node.branches) {
		if (branch.kind === "nil") continue;
		if (found !== undefined) return undefined;
		found = branch;
	}
	return found === undefined || found.kind === "variant" ? undefined : found;
}

/** @internal The descriptor below `node` at `segment`, or undefined below a leaf or a union of several types. */
export function childNode(node: Node | undefined, segment: Segment): Node | undefined {
	const at = tableBranch(node);
	if (at === undefined) return undefined;
	switch (at.kind) {
		case "object":
			if (!typeIs(segment, "string")) return undefined;
			for (const [name, child] of at.fields) if (name === segment) return child;
			return undefined;
		case "map":
			return typeIs(segment, "string") ? at.value : undefined;
		case "array":
			return typeIs(segment, "number") ? at.element : undefined;
		case "tuple":
			return typeIs(segment, "number") ? at.elements[segment] : undefined;
		default:
			return undefined;
	}
}

/** True when `a` sorts before `b` by bytes. */
function bytesLess(a: string, b: string): boolean {
	const length = math.min(a.size(), b.size());
	for (let i = 1; i <= length; i++) {
		const [x] = string.byte(a, i);
		const [y] = string.byte(b, i);
		if (x !== y) return x < y;
	}
	return a.size() < b.size();
}

/** @internal Rule 131's order: segment by segment, each by bytes, a prefix first. */
export function pathLess(a: readonly string[], b: readonly string[]): boolean {
	const length = math.min(a.size(), b.size());
	for (let i = 0; i < length; i++) if (a[i] !== b[i]) return bytesLess(a[i], b[i]);
	return a.size() < b.size();
}

/** @internal Two segment arrays are equal segment by segment, never as joined text (rules 4, 131). */
export function samePath(a: readonly string[], b: readonly string[]): boolean {
	if (a.size() !== b.size()) return false;
	for (let i = 0; i < a.size(); i++) if (a[i] !== b[i]) return false;
	return true;
}

/** `{ "career", "a.b" }`, as rule 131's messages print a path. */
export function showPath(path: readonly string[]): string {
	return `{ ${path.map((segment) => `"${segment}"`).join(", ")} }`;
}

/** A key that is "[]" or "*", or starts with a backslash, takes one more backslash, so no key stands for a marker. */
function escape(key: string): string {
	return key === "[]" || key === "*" || key.sub(1, 1) === "\\" ? `\\${key}` : key;
}

/**
 * @internal Rule 131: every object field path of `node`, one key per level, "[]" for an array element and "*" for a
 * map value, deduplicated across union members and sorted.
 */
export function fieldPaths(node: Node): readonly (readonly string[])[] {
	const found = new Map<string, string[]>();
	const add = (path: string[]) => {
		let identity = "";
		for (const segment of path) identity += `${segment.size()}:${segment}`;
		if (!found.has(identity)) found.set(identity, path);
	};
	const walk = (at: Branch, prefix: string[]) => {
		switch (at.kind) {
			case "object":
				for (const [key, child] of at.fields) {
					const path = [...prefix, escape(key)];
					add(path);
					walk(child, path);
				}
				return;
			case "array":
				return walk(at.element, [...prefix, "[]"]);
			case "tuple":
				for (const element of at.elements) walk(element, [...prefix, "[]"]);
				return;
			case "map":
				return walk(at.value, [...prefix, "*"]);
			case "union":
				for (const branch of at.branches) walk(branch, prefix);
				return;
			case "variant":
				add([...prefix, escape(at.key)]);
				return walk(at.body, prefix);
		}
	};
	walk(node, []);
	const paths = new Array<readonly string[]>();
	for (const [, path] of found) paths.push(table.freeze(path));
	paths.sort(pathLess);
	return table.freeze(paths);
}
