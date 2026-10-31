export type Literal = string | number | boolean;
export type Node =
	| {
			readonly kind: "nil";
	  }
	| {
			readonly kind: "bool" | "f64" | "f32" | "u8" | "u16" | "u32" | "string";
	  }
	| {
			readonly kind: "vector3" | "color3" | "cframe";
	  }
	| {
			readonly kind: "literal";
			readonly value: Literal;
	  }
	| {
			readonly kind: "instance";
			readonly classes: readonly (keyof Instances)[];
	  }
	| {
			readonly kind: "object";
			readonly fields: readonly (readonly [name: string, value: Node])[];
	  }
	| {
			readonly kind: "array";
			readonly element: Node;
	  }
	| {
			readonly kind: "tuple";
			readonly elements: readonly Node[];
	  }
	| {
			readonly kind: "map";
			readonly key: MapKey;
			readonly value: Node;
	  }
	| {
			readonly kind: "union";
			readonly branches: readonly Node[];
	  }
	| {
			readonly kind: "variant";
			readonly key: string;
			readonly value: Literal;
			readonly body: Node;
	  }
	| {
			readonly kind: "dynamic";
	  };
export type MapKey =
	| {
			readonly kind: "string" | "u8" | "u16" | "u32";
	  }
	| {
			readonly kind: "union";
			readonly branches: readonly {
				readonly kind: "literal";
				readonly value: string | number;
			}[];
	  };
export type Kind<K extends Node["kind"]> =
	Extract<Node, { readonly kind: K }> extends never ? { readonly kind: K } : Extract<Node, { readonly kind: K }>;
export interface LiteralNode<V extends Literal> {
	readonly kind: "literal";
	readonly value: V;
}
export interface UnionNode<B extends Node> {
	readonly kind: "union";
	readonly branches: readonly B[];
}
export interface NodeBuilders {
	readonly nil: { readonly kind: "nil" };
	readonly bool: { readonly kind: "bool" };
	readonly f64: { readonly kind: "f64" };
	readonly f32: { readonly kind: "f32" };
	readonly u8: { readonly kind: "u8" };
	readonly u16: { readonly kind: "u16" };
	readonly u32: { readonly kind: "u32" };
	readonly string: { readonly kind: "string" };
	readonly vector3: { readonly kind: "vector3" };
	readonly color3: { readonly kind: "color3" };
	readonly cframe: { readonly kind: "cframe" };
	readonly dynamic: { readonly kind: "dynamic" };
	readonly literal: <V extends Literal>(value: V) => LiteralNode<V>;
	readonly instance: (classes: readonly (keyof Instances)[]) => Kind<"instance">;
	readonly object: (fields: readonly (readonly [name: string, value: Node])[]) => Kind<"object">;
	readonly array: (element: Node) => Kind<"array">;
	readonly tuple: (elements: readonly Node[]) => Kind<"tuple">;
	readonly map: (key: MapKey, value: Node) => Kind<"map">;
	readonly union: <B extends Node>(branches: readonly B[]) => UnionNode<B>;
	readonly variant: (key: string, value: Literal, body: Node) => Kind<"variant">;
}
export declare const n: NodeBuilders;
