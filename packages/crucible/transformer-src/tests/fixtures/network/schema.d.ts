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
export interface NodeBuilders {
	readonly nil: Node;
	readonly bool: Node;
	readonly f64: Node;
	readonly f32: Node;
	readonly u8: Node;
	readonly u16: Node;
	readonly u32: Node;
	readonly string: Node;
	readonly vector3: Node;
	readonly color3: Node;
	readonly cframe: Node;
	readonly dynamic: Node;
	readonly literal: (value: Literal) => Node;
	readonly instance: (classes: readonly (keyof Instances)[]) => Node;
	readonly object: (fields: readonly (readonly [name: string, value: Node])[]) => Node;
	readonly array: (element: Node) => Node;
	readonly tuple: (elements: readonly Node[]) => Node;
	readonly map: (key: MapKey, value: Node) => Node;
	readonly union: (branches: readonly Node[]) => Node;
	readonly variant: (key: string, value: Literal, body: Node) => Node;
}
export declare const n: NodeBuilders;
