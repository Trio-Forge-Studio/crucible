import type { Subsystem } from "../subsystem";
import type { CrucibleObject } from "../crucible-object";

/** @internal */
export const enum FrameHook {
	PreRender,
	FixedUpdate,
	Update,
	LateUpdate,
}

/** @internal */
export type HookMask = number;

/** @internal */
export const FRAME_HOOK_NAMES = ["preRender", "fixedUpdate", "update", "lateUpdate"] as const;

/** @internal */
export const OBJECT_HOOK_NAMES = ["awake", "start", "fixedUpdate", "update", "lateUpdate", "preRender"] as const;

/** @internal */
export const BEHAVIOUR_HOOK_NAMES = ["onEnable", "onDisable", "onDestroy"] as const;

/** @internal */
export interface ObjectHooks {
	readonly awake: (o: CrucibleObject, dt: number) => void;
	readonly start: (o: CrucibleObject, dt: number) => void;
	readonly frame: { readonly [H in FrameHook]: (o: CrucibleObject, dt: number) => void };
	/** @internal */
	readonly overrides: (klass: object) => HookMask;
}

/** @internal */
export interface BehaviourHooks {
	readonly onEnable: (b: CrucibleObject, dt: number) => void;
	readonly onDisable: (b: CrucibleObject, dt: number) => void;
	readonly onDestroy: (b: CrucibleObject, dt: number) => void;
}

/** @internal */
export const SUBSYSTEM_HOOK_NAMES = ["onShutdown"] as const;

/** @internal */
export interface SubsystemHooks {
	readonly onShutdown: (o: Subsystem) => void;
}
let subsystemHooks: SubsystemHooks | undefined;
/** @internal */
export function installSubsystemHooks(value: SubsystemHooks) {
	subsystemHooks = value;
}
/** @internal */
export function shutdownHooks(): SubsystemHooks {
	return subsystemHooks ?? error("Subsystem hooks not installed");
}

let objectHooks: ObjectHooks | undefined;
let behaviourHooks: BehaviourHooks | undefined;

/** @internal */
export function installObjectHooks(hooks: ObjectHooks) {
	objectHooks = hooks;
}

/** @internal */
export function installBehaviourHooks(hooks: BehaviourHooks) {
	behaviourHooks = hooks;
}

/** @internal */
export function hooks(): ObjectHooks {
	return objectHooks ?? error("Crucible hooks used before CrucibleObject loaded");
}

/** @internal */
export function lifecycleHooks(): BehaviourHooks {
	return behaviourHooks ?? error("Crucible hooks used before CrucibleObject loaded");
}
