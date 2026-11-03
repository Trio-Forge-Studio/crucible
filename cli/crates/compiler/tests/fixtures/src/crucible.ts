import { inject, Subsystem } from "@trioforge/crucible";
import { attribute as emitAttribute } from "@trioforge/crucible/emit";
import { boot } from "@trioforge/crucible/server";

export class Store extends Subsystem {}
export const store = inject<Store>();
export const untyped = inject();
const alias = inject;
export const aliased = alias<Store>();
export const fallback = emitAttribute({}, "speed", 1);
boot({ load: [] });
