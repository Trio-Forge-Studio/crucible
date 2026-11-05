import type { Migration, OperationId, ProfileMessage, StoreFailure, StoreResult } from "../data";

declare const recordKind: unique symbol;

export interface StoreOptions<T> {
	readonly defaults: T;
	readonly migrations?: readonly Migration[];
}

export type UpdateResult<T> =
	| { readonly ok: true; readonly value: T; readonly replayed: boolean }
	| { readonly ok: false; readonly reason: StoreFailure };

export interface DataStore<T> {
	readonly [recordKind]: { readonly kind: "records"; readonly value: (value: T) => T };

	read(key: string): StoreResult<T | undefined>;
	update(key: string, change: (current: T) => T, id?: OperationId): UpdateResult<T>;
}

export declare function dataStore<T extends object>(name: string, options: StoreOptions<NoInfer<T>>): DataStore<T>;
export declare function profileMessage<P>(name: string): ProfileMessage<P>;

export declare function operationId(): OperationId;

import type { Readable } from "../state";

declare const configKind: unique symbol;

export interface Config<T> extends Readable<T> {
	readonly [configKind]: { readonly scope: "global"; readonly value: (value: T) => T };
}

export interface PlayerConfig<T> {
	(player: Player): T;
	readonly [configKind]: { readonly scope: "player"; readonly value: (value: T) => T };
}

export declare function config<T = never>(name: string, fallback: NoInfer<T>): Config<T>;
export declare function playerConfig<T = never>(name: string, fallback: NoInfer<T>): PlayerConfig<T>;

export declare function configReady(player?: Player): boolean;

export declare function refreshConfigs(): boolean;
