import { claimPublication } from "../internal/sync/declarations";
import type { Replicated, ReplicatedPerPlayer } from "../replicated";

declare const publication: unique symbol;

/** What a per-player `publish` returns: an opaque claim that holds nothing callable. */
export interface Publication {
	readonly [publication]: true;
}

/**
 * What a public `publish` returns, read and written like a Charm atom. `h()` reads, tracked. `h(value)` writes a value,
 * and `h(update)` writes what `update` returns for the current value, called untracked. Write `undefined` with
 * `h(() => undefined)`: a call cannot tell `h(undefined)` from `h()`, so the type refuses it.
 */
export interface Published<T> {
	(): T;
	(update: Exclude<T, undefined> | ((current: T) => T)): T;
}

/**
 * Takes the write side of a value every client sees. Call it while a server Subsystem is constructed, as a field
 * initializer: `private readonly phase = publish(JobPhase)`. A write drops fields the type does not declare, checks
 * and freezes the rest, and throws when it cannot replicate. One Subsystem publishes each value.
 */
export function publish<T>(target: Replicated<T>): Published<T>;
/**
 * Supplies a per-player value as a projection. For each player whose client is ready, the Heartbeat that examines
 * the player evaluates it once what it last read has changed, admits the result and sends it to that player alone.
 * It must be pure and must not yield. A throw or a rejected value marks that player's value failed until it next
 * changes.
 */
export function publish<T>(target: ReplicatedPerPlayer<T>, project: (player: Player) => T): Publication;
export function publish(target: unknown, project?: (player: Player) => unknown): unknown {
	return claimPublication(target, project);
}
