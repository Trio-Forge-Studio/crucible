import * as __crucible from "@trioforge/crucible/emit";

export const f32Key = __crucible.n.map(__crucible.n.f32, __crucible.n.u8);
export const f64Key = __crucible.n.map(__crucible.n.f64, __crucible.n.u8);
export const booleanLiteralKey = __crucible.n.map(__crucible.n.union([__crucible.n.literal(true)]), __crucible.n.u8);
export const optionalKey = __crucible.n.map(
	__crucible.n.union([__crucible.n.nil, __crucible.n.literal("a")]),
	__crucible.n.u8,
);
export const variantOutsideUnion = __crucible.n.array(__crucible.n.variant("Kind", "A", __crucible.n.object([])));
export const scalarVariantBody = __crucible.n.variant("Kind", "A", __crucible.n.u8);
