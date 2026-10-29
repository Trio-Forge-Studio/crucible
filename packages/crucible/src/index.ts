export { Subsystem } from "./subsystem";
export type { CrucibleObject } from "./crucible-object";
export type { All } from "./all";
export { inject } from "./inject";
export { attribute } from "./attribute";
export { CrucibleBehaviour, type BehaviourClass } from "./crucible-behaviour";

export interface BootOptions {
	readonly load: readonly string[];
	readonly fixedRate?: number;
}
