import { ready, refresh } from "./internal/configs/runtime";

declare const configKind: unique symbol;

/** An experience-wide config. Calling it reads the global snapshot's value; it never yields or throws. */
export interface Config<T> {
	(): T;
	readonly [configKind]: { readonly scope: "global"; readonly value: (value: T) => T };
}

/** A targeted config. Calling it reads that player's value; it never yields or throws. */
export interface PlayerConfig<T> {
	(player: Player): T;
	readonly [configKind]: { readonly scope: "player"; readonly value: (value: T) => T };
}

/**
 * Declares an experience config by its dashboard key, read from the global snapshot. It must initialize an exported
 * top-level const in a server module, with `T` explicit. The fallback is the value before the first fetch and for a
 * missing key.
 */
export function config<T = never>(_name: string, _fallback: NoInfer<T>): Config<T> {
	error("Crucible configs need the @trioforge/crucible/transformer plugin");
}

/** Declares a targeted config by its dashboard key, read from each player's snapshot. Same form as `config`. */
export function playerConfig<T = never>(_name: string, _fallback: NoInfer<T>): PlayerConfig<T> {
	error("Crucible configs need the @trioforge/crucible/transformer plugin");
}

/**
 * With no argument, true once the global snapshot has been applied. With a player, true once that player's snapshot
 * has been applied, and false before and after they leave.
 */
export function configReady(player?: Player): boolean {
	return ready(player);
}

/**
 * Refreshes every snapshot with newer values, fetches every snapshot that has not loaded, and applies them all in
 * one pass. Yields, at most 10 seconds. True when every snapshot is current; false keeps the last good values.
 * Concurrent calls share one refresh. Crucible never calls it.
 */
export function refreshConfigs(): boolean {
	return refresh();
}
