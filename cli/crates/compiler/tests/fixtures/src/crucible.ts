import { inject, Subsystem } from "@trioforge/crucible";

export class Store extends Subsystem {}
export const store = inject<Store>();
