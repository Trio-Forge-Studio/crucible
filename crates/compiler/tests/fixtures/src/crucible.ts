import { attribute as emitAttribute } from "@trioforge/crucible/emit";
import { boot } from "@trioforge/crucible/server";

export const fallback = emitAttribute({}, "speed", 1);
boot({ load: [] });
