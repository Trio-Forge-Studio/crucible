import { MAX_UNION_BRANCHES } from "./limits";

export type Literal = string | number | boolean;
export type Node =
	| {
			readonly kind:
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
	  }
	| { readonly kind: "literal"; readonly value: Literal }
	| { readonly kind: "instance"; readonly classes: readonly (keyof Instances)[] }
	| { readonly kind: "object"; readonly fields: readonly (readonly [string, Node])[] }
	| { readonly kind: "array"; readonly element: Node }
	| { readonly kind: "tuple"; readonly elements: readonly Node[] }
	| { readonly kind: "map"; readonly key: Node; readonly value: Node }
	| { readonly kind: "union"; readonly branches: readonly Node[] }
	| { readonly kind: "variant"; readonly key: string; readonly value: Literal; readonly body: Node };

function leaf<K extends Node["kind"]>(kind: K): { readonly kind: K } {
	return table.freeze({ kind });
}

function literal(value: Literal): Node {
	assert(
		typeIs(value, "string") ||
			typeIs(value, "boolean") ||
			(typeIs(value, "number") && value === value && math.abs(value) < math.huge),
		"invalid literal",
	);
	return table.freeze({ kind: "literal", value });
}

export function admitsNil(node: Node): boolean {
	return node.kind === "nil" || node.kind === "dynamic" || (node.kind === "union" && node.branches.some(admitsNil));
}

function assertNode(node: Node) {
	assert(typeIs(node, "table") && table.isfrozen(node), "expected a descriptor constructor result");
}

export function runtimeKind(node: Node): string {
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

export const n = table.freeze({
	nil: leaf("nil"),
	bool: leaf("bool"),
	f64: leaf("f64"),
	f32: leaf("f32"),
	u8: leaf("u8"),
	u16: leaf("u16"),
	u32: leaf("u32"),
	string: leaf("string"),
	vector3: leaf("vector3"),
	color3: leaf("color3"),
	cframe: leaf("cframe"),
	dynamic: leaf("dynamic"),
	literal,
	instance: (classes: readonly (keyof Instances)[]): Node => {
		assert(classes.size() > 0, "empty Instance classes");
		const copy = [...classes];
		for (let i = 0; i < copy.size(); i++)
			assert(
				typeIs(copy[i], "string") && (i === 0 || copy[i - 1] < copy[i]),
				"Instance classes must be sorted and unique",
			);
		return table.freeze({ kind: "instance", classes: table.freeze(copy) });
	},
	object: (fields: readonly (readonly [string, Node])[]): Node => {
		const copy = fields.map(([name, node]) => {
			assertNode(node);
			assert(typeIs(name, "string") && tostring(tonumber(name)) !== name, "numeric property name");
			return table.freeze([name, node] as [string, Node]);
		});
		for (let i = 1; i < copy.size(); i++) assert(copy[i - 1][0] < copy[i][0], "fields must be sorted and unique");
		return table.freeze({ kind: "object", fields: table.freeze(copy) });
	},
	array: (element: Node): Node => {
		assertNode(element);
		assert(!admitsNil(element), "nil array element");
		return table.freeze({ kind: "array", element });
	},
	tuple: (elements: readonly Node[]): Node => {
		for (const node of elements) {
			assertNode(node);
			assert(!admitsNil(node), "nil tuple element");
		}
		return table.freeze({ kind: "tuple", elements: table.freeze([...elements]) });
	},
	map: (key: Node, value: Node): Node => {
		assertNode(key);
		assertNode(value);
		assert(
			key.kind === "string" ||
				key.kind === "u8" ||
				key.kind === "u16" ||
				key.kind === "u32" ||
				(key.kind === "union" &&
					key.branches.every((b) => b.kind === "literal" && !typeIs(b.value, "boolean"))),
			"invalid map key",
		);
		assert(!admitsNil(value), "nil map value");
		return table.freeze({ kind: "map", key, value });
	},
	union: (branches: readonly Node[]): Node => {
		assert(branches.size() > 0 && branches.size() <= MAX_UNION_BRANCHES, "invalid union size");
		const kinds = new Set<string>();
		let phase = 0;
		let previousLiteral: Literal | undefined;
		let previousKind: string | undefined;
		let discriminant: string | undefined;
		for (let i = 0; i < branches.size(); i++) {
			const branch = branches[i];
			assertNode(branch);
			if (branch.kind === "nil") {
				assert(i === 0, "absent must be first");
				continue;
			}
			if (branch.kind === "literal") {
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
		return table.freeze({ kind: "union", branches: table.freeze([...branches]) });
	},
	variant: (key: string, value: Literal, body: Node): Node => {
		assertNode(body);
		literal(value);
		assert(
			typeIs(key, "string") &&
				tostring(tonumber(key)) !== key &&
				body.kind === "object" &&
				body.fields.every(([name]) => name !== key),
			"invalid variant",
		);
		return table.freeze({ kind: "variant", key, value, body });
	},
});

export type NodeBuilders = typeof n;
