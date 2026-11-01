declare const replicatedKind: unique symbol;

export interface Readable<T> {
	(): T;
}
export type SyncStatus = "pending" | "ready" | "failed";
export interface Replicated<T> extends Readable<T> {
	readonly [replicatedKind]: { readonly audience: "all"; readonly value: (value: T) => T };
}
export interface ReplicatedPerPlayer<T> extends Readable<T> {
	readonly [replicatedKind]: { readonly audience: "player"; readonly value: (value: T) => T };
}
export declare function replicated<T = never>(name: string, initial: NoInfer<T>): Replicated<T>;
export declare function replicatedPerPlayer<T = never>(name: string, initial: NoInfer<T>): ReplicatedPerPlayer<T>;
export declare function syncStatus<T>(declaration: Replicated<T> | ReplicatedPerPlayer<T>): SyncStatus;
