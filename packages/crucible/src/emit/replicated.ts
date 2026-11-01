import type { Node } from "../internal/network/schema";
import { declareReplicated } from "../internal/sync/declarations";
import type { Replicated, ReplicatedPerPlayer } from "../replicated";

export function replicated<T>(name: string, initial: NoInfer<T>, value: Node): Replicated<T> {
	return declareReplicated("all", name, initial, value) as Replicated<T>;
}

export function replicatedPerPlayer<T>(name: string, initial: NoInfer<T>, value: Node): ReplicatedPerPlayer<T> {
	return declareReplicated("player", name, initial, value) as ReplicatedPerPlayer<T>;
}
