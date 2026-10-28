import type { All, CrucibleObject } from "./index";

export type Check = (value: unknown) => boolean;
export type EmittedDep = string | { readonly all: string };
interface EmittedBase {
	readonly id: string;
	readonly deps: readonly EmittedDep[];
	readonly provides: readonly string[];
}
export interface EmittedAttribute {
	readonly name: string;
	readonly check: Check;
	readonly required: boolean;
}
export type EmittedClass =
	| (EmittedBase & { readonly kind: "subsystem" })
	| (EmittedBase & {
			readonly kind: "behaviour";
			readonly tag: string;
			readonly instance: readonly (keyof Instances)[];
			readonly attributes?: { readonly [field: string]: EmittedAttribute };
	  });
export declare function defineClass(klass: new (...args: never[]) => CrucibleObject, emitted: EmittedClass): void;
export declare function resolve<T extends object>(id: string): T;
export declare function resolveAll<T extends object>(id: string): All<T>;
export declare function attribute<T>(object: object, field: string, fallback?: T): T;
export declare const t: {
	readonly string: Check;
	readonly number: Check;
	readonly boolean: Check;
	readonly vector3: Check;
	readonly color3: Check;
	readonly literal: (expected: string | number | boolean) => Check;
	readonly optional: (check: Check) => Check;
	readonly union: (...checks: Check[]) => Check;
};
