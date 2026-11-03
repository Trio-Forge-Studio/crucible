import { claimPublication } from "../internal/sync/declarations";
import type { State } from "../reactive";
import type { Replicated, ReplicatedPerPlayer } from "../replicated";

declare const publication: unique symbol;

/** What a per-player `publish` returns: an opaque claim that holds nothing callable. */
export interface Publication {
	readonly [publication]: true;
}

/**
 * Takes the write side of a value every client sees. Call it while a server Subsystem is constructed, as a field
 * initializer: `private readonly phase = publish(JobPhase)`. `set` and `update` drop fields the type does not
 * declare, check and freeze the rest, and throw at a write that cannot replicate. One Subsystem publishes each value.
 */
export function publish<T>(target: Replicated<T>): State<T>;
/**
 * Supplies a per-player value as a projection. For each player whose client is ready, it is evaluated like a
 * `computed`, admitted on every pull and sent to that player alone. It must be pure and must not yield. A throw or a
 * rejected value marks that player's value failed until it next changes.
 */
export function publish<T>(target: ReplicatedPerPlayer<T>, project: (player: Player) => T): Publication;
export function publish(target: unknown, project?: (player: Player) => unknown): unknown {
	return claimPublication(target, project);
}
