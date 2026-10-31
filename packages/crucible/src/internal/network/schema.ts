import { MAX_UNION_BRANCHES } from "./limits";

export type Literal = string | number | boolean;
export type LeafKind =
	| "nil"
	| "bool"
	| "f64"
	| "f32"
	| "u8"
	| "u16"
	| "u32"
	| "string"
	| "vector3"
	| "color3"
	| "cframe"
	| "dynamic";
export interface LeafNode<K extends LeafKind = LeafKind> {
	readonly kind: K;
}
export interface LiteralNode<V extends Literal = Literal> {
	readonly kind: "literal";
	readonly value: V;
}
export interface InstanceNode {
	readonly kind: "instance";
	readonly classes: readonly (keyof Instances)[];
}
export interface ObjectNode {
	readonly kind: "object";
	readonly fields: readonly (readonly [name: string, value: Node])[];
}
export interface ArrayNode {
	readonly kind: "array";
	readonly element: Node;
}
export interface TupleNode {
	readonly kind: "tuple";
	readonly elements: readonly Node[];
}
export interface MapNode {
	readonly kind: "map";
	readonly key: MapKey;
	readonly value: Node;
}
export interface VariantNode {
	readonly kind: "variant";
	readonly key: string;
	readonly value: Literal;
	readonly body: ObjectNode;
}
export type Branch = Node | VariantNode;
export interface UnionNode<B extends Branch = Branch> {
	readonly kind: "union";
	readonly branches: readonly B[];
}
export type Node = LeafNode | LiteralNode | InstanceNode | ObjectNode | ArrayNode | TupleNode | MapNode | UnionNode;
export type MapKey = LeafNode<"string" | "u8" | "u16" | "u32"> | UnionNode<LiteralNode<string | number>>;

export interface NodeBuilders {
	readonly nil: LeafNode<"nil">;
	readonly bool: LeafNode<"bool">;
	readonly f64: LeafNode<"f64">;
	readonly f32: LeafNode<"f32">;
	readonly u8: LeafNode<"u8">;
	readonly u16: LeafNode<"u16">;
	readonly u32: LeafNode<"u32">;
	readonly string: LeafNode<"string">;
	readonly vector3: LeafNode<"vector3">;
	readonly color3: LeafNode<"color3">;
	readonly cframe: LeafNode<"cframe">;
	readonly dynamic: LeafNode<"dynamic">;
	readonly literal: <V extends Literal>(value: V) => LiteralNode<V>;
	readonly instance: (classes: readonly (keyof Instances)[]) => InstanceNode;
	readonly object: (fields: readonly (readonly [name: string, value: Node])[]) => ObjectNode;
	readonly array: (element: Node) => ArrayNode;
	readonly tuple: (elements: readonly Node[]) => TupleNode;
	readonly map: (key: MapKey, value: Node) => MapNode;
	readonly union: <B extends Branch>(branches: readonly B[]) => UnionNode<B>;
	readonly variant: (key: string, value: Literal, body: ObjectNode) => VariantNode;
}

// Descriptors arrive from Luau, where the types above do not hold, so provenance is checked at runtime.
const descriptors = new Set<object>();

function descriptor<T extends Branch>(node: T): T {
	descriptors.add(node);
	return table.freeze(node) as T;
}

export function assertDescriptor(node: unknown): asserts node is Branch {
	assert(typeIs(node, "table") && descriptors.has(node), "expected a descriptor constructor result");
}

function numericName(name: string) {
	return tostring(tonumber(name)) === name;
}

function literal<V extends Literal>(value: V): LiteralNode<V> {
	assert(
		typeIs(value, "string") ||
			typeIs(value, "boolean") ||
			(typeIs(value, "number") && value === value && math.abs(value) < math.huge),
		"invalid literal",
	);
	return descriptor({ kind: "literal", value });
}

export function admitsNil(node: Branch): boolean {
	return node.kind === "nil" || node.kind === "dynamic" || (node.kind === "union" && node.branches.some(admitsNil));
}

export function runtimeKind(node: Branch): string {
	switch (node.kind) {
		case "bool":
			return "boolean";
		case "f64":
		case "f32":
		case "u8":
		case "u16":
		case "u32":
			return "number";
		case "string":
			return "string";
		case "vector3":
			return "Vector3";
		case "color3":
			return "Color3";
		case "cframe":
			return "CFrame";
		case "instance":
			return "Instance";
		default:
			return "table";
	}
}

export function literalLess(a: Literal, b: Literal): boolean {
	const ranks = { boolean: 0, number: 1, string: 2 };
	const ak = typeOf(a) as keyof typeof ranks;
	const bk = typeOf(b) as keyof typeof ranks;
	if (ak !== bk) return ranks[ak] < ranks[bk];
	if (typeIs(a, "boolean") && typeIs(b, "boolean")) return !a && b;
	if (typeIs(a, "number") && typeIs(b, "number")) return a < b;
	return (a as string) < (b as string);
}

/** Rule 63 order and rule 65 ambiguity, asserted by the builder and again by `compile`. */
export function checkUnion(branches: readonly Branch[]) {
	assert(branches.size() > 0 && branches.size() <= MAX_UNION_BRANCHES, "invalid union size");
	const kinds = new Set<string>();
	let phase = 0;
	let previousLiteral: Literal | undefined;
	let previousKind: string | undefined;
	let discriminant: string | undefined;
	for (let i = 0; i < branches.size(); i++) {
		const branch = branches[i];
		assertDescriptor(branch);
		if (branch.kind === "nil") {
			assert(i === 0, "absent must be first");
		} else if (branch.kind === "literal") {
			assert(
				phase <= 1 && (previousLiteral === undefined || literalLess(previousLiteral, branch.value)),
				"literal order",
			);
			phase = 1;
			previousLiteral = branch.value;
		} else if (branch.kind === "variant") {
			assert(
				!kinds.has("table") && (discriminant === undefined || discriminant === branch.key),
				"ambiguous variants",
			);
			if (phase < 3) previousLiteral = undefined;
			assert(previousLiteral === undefined || literalLess(previousLiteral, branch.value), "variant order");
			phase = 3;
			previousLiteral = branch.value;
			discriminant = branch.key;
		} else {
			const kind = runtimeKind(branch);
			assert(
				branch.kind !== "union" &&
					branch.kind !== "dynamic" &&
					phase <= 2 &&
					!kinds.has(kind) &&
					(previousKind === undefined || previousKind < kind),
				"ambiguous union",
			);
			phase = 2;
			kinds.add(kind);
			previousKind = kind;
		}
	}
}

function node(child: unknown): Node {
	assertDescriptor(child);
	assert(child.kind !== "variant", "variant outside union");
	return child;
}

export const n: NodeBuilders = table.freeze({
	nil: descriptor({ kind: "nil" }),
	bool: descriptor({ kind: "bool" }),
	f64: descriptor({ kind: "f64" }),
	f32: descriptor({ kind: "f32" }),
	u8: descriptor({ kind: "u8" }),
	u16: descriptor({ kind: "u16" }),
	u32: descriptor({ kind: "u32" }),
	string: descriptor({ kind: "string" }),
	vector3: descriptor({ kind: "vector3" }),
	color3: descriptor({ kind: "color3" }),
	cframe: descriptor({ kind: "cframe" }),
	dynamic: descriptor({ kind: "dynamic" }),
	literal,
	instance: (classes) => {
		assert(typeIs(classes, "table") && classes.size() > 0, "empty Instance classes");
		const copy = [...classes];
		for (let i = 0; i < copy.size(); i++)
			assert(
				typeIs(copy[i], "string") && (i === 0 || copy[i - 1] < copy[i]),
				"Instance classes must be sorted and unique",
			);
		return descriptor({ kind: "instance", classes: table.freeze(copy) });
	},
	object: (fields) => {
		const copy = fields.map(([name, child]) => {
			assert(typeIs(name, "string") && !numericName(name), "numeric property name");
			return table.freeze([name, node(child)] as const);
		});
		for (let i = 1; i < copy.size(); i++) assert(copy[i - 1][0] < copy[i][0], "fields must be sorted and unique");
		return descriptor({ kind: "object", fields: table.freeze(copy) });
	},
	array: (element) => {
		assert(!admitsNil(node(element)), "nil array element");
		return descriptor({ kind: "array", element });
	},
	tuple: (elements) => {
		const copy = elements.map((element) => {
			assert(!admitsNil(node(element)), "nil tuple element");
			return element;
		});
		return descriptor({ kind: "tuple", elements: table.freeze(copy) });
	},
	map: (key, value) => {
		node(key);
		assert(
			key.kind === "string" ||
				key.kind === "u8" ||
				key.kind === "u16" ||
				key.kind === "u32" ||
				(key.kind === "union" &&
					key.branches.every((b) => b.kind === "literal" && !typeIs(b.value, "boolean"))),
			"invalid map key",
		);
		assert(!admitsNil(node(value)), "nil map value");
		return descriptor({ kind: "map", key, value });
	},
	union: <B extends Branch>(branches: readonly B[]) => {
		checkUnion(branches);
		return descriptor({ kind: "union", branches: table.freeze([...branches]) });
	},
	variant: (key, value, body) => {
		literal(value);
		assert(
			typeIs(key, "string") &&
				!numericName(key) &&
				node(body).kind === "object" &&
				body.fields.every(([name]) => name !== key),
			"invalid variant",
		);
		return descriptor({ kind: "variant", key, value, body });
	},
});
