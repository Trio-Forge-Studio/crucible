type AttributeValue = string | number | boolean | Vector3 | Color3;

declare abstract class CrucibleObject {
	static readonly renderPriority?: number;
	protected awake(): void;
	protected start(): void;
	protected fixedUpdate(step: number): void;
	protected update(dt: number): void;
	protected lateUpdate(dt: number): void;
	protected preRender(dt: number): void;
}

export type { CrucibleObject };

export abstract class Subsystem extends CrucibleObject {
	protected onShutdown(): void;
}

export abstract class CrucibleBehaviour<Tag extends string, I extends Instance = Instance> extends CrucibleObject {
	static accepts(instance: Instance): boolean;
	static refresh(instance: Instance): void;
	static get<T extends CrucibleBehaviour<string>>(this: BehaviourClass<T>, instance: Instance): T | undefined;
	readonly instance: I;
	readonly tag: Tag;
	isEnabled(): boolean;
	setEnabled(enabled: boolean): void;
	remove(): void;
	protected onEnable(): void;
	protected onDisable(): void;
	protected onDestroy(): void;
}

export type BehaviourClass<T extends CrucibleBehaviour<string>> = new (...args: never[]) => T;

export interface All<T extends object> {
	size(): number;
	has(item: T): boolean;
	first(): T | undefined;
	forEach(visit: (item: T) => void): void;
	observe(added: (item: T) => void, removed?: (item: T) => void): () => void;
}

export function inject<T extends object>(): T;

export function attribute<T extends AttributeValue>(): T;
export function attribute(fallback: number): number;
export function attribute(fallback: string): string;
export function attribute(fallback: boolean): boolean;
export function attribute<T extends AttributeValue | undefined>(fallback: T): T;

export interface BootOptions {
	readonly load: readonly string[];
	readonly fixedRate?: number;
}
