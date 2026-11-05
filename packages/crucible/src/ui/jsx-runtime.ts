import type { Readable } from "../reactive";
import type { Node } from ".";

/** An instance's writable properties. Each takes a value, or a `Readable` that the element binds. */
export type PropertyProps<T extends Instance> = {
	[K in keyof WritableInstanceProperties<T>]?:
		| WritableInstanceProperties<T>[K]
		| Readable<WritableInstanceProperties<T>[K]>;
};

/** An instance's events. Each takes a handler that the element connects. */
export type EventProps<T extends Instance> = {
	[K in InstanceEventNames<T>]?: T[K] extends RBXScriptSignal<infer Handler> ? Handler : never;
};

/** The props of an intrinsic element that creates a `T`. */
export type IntrinsicProps<T extends Instance> = PropertyProps<T> & EventProps<T> & { children?: Node };

export declare namespace JSX {
	type Element = Node;
	type ElementType = keyof IntrinsicElements | ((props: never) => Node);
	interface ElementChildrenAttribute {
		children: unknown;
	}
	type IntrinsicElements = {
		[K in keyof CreatableInstances as Lowercase<K>]: IntrinsicProps<CreatableInstances[K]>;
	};
}
