import { readClaim } from "../internal/construction-claim";
export { n } from "../internal/network/schema";
export { config, playerConfig } from "./configs";
export { toServer, toClient, askServer, receive } from "./network";

import type { All } from "../all";
import type { CrucibleObject } from "../crucible-object";
import { define, resolveOne, setFor, type TypeId } from "../internal/registry";

export type EmittedDep = string | { readonly all: string };

interface EmittedBase {
	readonly id: string;

	readonly deps: readonly EmittedDep[];

	readonly provides: readonly string[];
}

export type EmittedClass =
	| (EmittedBase & { readonly kind: "subsystem" })
	| (EmittedBase & {
			readonly kind: "behaviour";
			readonly tag: string;

			readonly instance: readonly (keyof Instances)[];

			readonly attributes?: { readonly [field: string]: EmittedAttribute };
	  });

export type Check = (value: unknown) => boolean;

export function defineClass(klass: new (...args: never[]) => CrucibleObject, emitted: EmittedClass) {
	define(klass, emitted);
}

export function resolve<T extends object>(id: string): T {
	return resolveOne(id as TypeId) as unknown as T;
}

export function resolveAll<T extends object>(id: string): All<T> {
	return setFor(id as TypeId) as unknown as All<T>;
}

export const t: {
	readonly string: Check;
	readonly number: Check;
	readonly boolean: Check;
	readonly vector3: Check;
	readonly color3: Check;
	readonly literal: (expected: string | number | boolean) => Check;
	readonly optional: (check: Check) => Check;
	readonly union: (...checks: Check[]) => Check;
} = {
	string: ((value) => typeIs(value, "string")) as Check,
	number: ((value) => typeIs(value, "number")) as Check,
	boolean: ((value) => typeIs(value, "boolean")) as Check,
	vector3: ((value) => typeIs(value, "Vector3")) as Check,
	color3: ((value) => typeIs(value, "Color3")) as Check,
	literal:
		(expected: string | number | boolean): Check =>
		(value) =>
			value === expected,
	optional:
		(check: Check): Check =>
		(value) =>
			value === undefined || check(value),
	union:
		(...checks: Check[]): Check =>
		(value) =>
			checks.some((check) => check(value)),
};

export interface EmittedAttribute {
	readonly name: string;
	readonly check: Check;
	readonly required: boolean;
}
export function attribute<T>(object: object, field: string, fallback?: T): T {
	const claim = readClaim();
	if (claim === undefined || claim.object !== object) error("attribute read outside construction claim");
	const value = claim.attributes?.[field];
	return (value === undefined ? fallback : value) as T;
}
