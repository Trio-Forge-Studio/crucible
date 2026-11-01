import { Replicated, ReplicatedPerPlayer } from "../state";
import { ProfileOptions, ProfileStore, ProfileMessage } from "../data";
import { Config, DataStore, PlayerConfig, StoreOptions } from "../server/data";
import { Node } from "./schema";
export declare function replicated<T>(name: string, initial: NoInfer<T>, value: Node): Replicated<T>;
export declare function replicatedPerPlayer<T>(name: string, initial: NoInfer<T>, value: Node): ReplicatedPerPlayer<T>;
export declare function profileStore<T extends object>(
	name: string,
	options: ProfileOptions<T>,
	persist: Node,
	replica: Node,
	serverOnlyPaths: readonly (readonly string[])[],
): ProfileStore<T>;
export declare function dataStore<T extends object>(
	name: string,
	options: StoreOptions<NoInfer<T>>,
	value: Node,
): DataStore<T>;
export declare function profileMessage<P>(name: string, payload: Node): ProfileMessage<P>;
export declare function config<T>(name: string, fallback: NoInfer<T>, value: Node): Config<T>;
export declare function playerConfig<T>(name: string, fallback: NoInfer<T>, value: Node): PlayerConfig<T>;
