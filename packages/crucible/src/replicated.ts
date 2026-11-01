import { statusOf } from "./internal/sync/declarations";
import type { Readable } from "./reactive";

declare const replicatedKind: unique symbol;

/** Whether a replicated value holds what the server published: before the first frame, after it, or failed. */
export type SyncStatus = "pending" | "ready" | "failed";

/**
 * A value every client receives. On the server it reads the authoritative value; on a client, the last value the
 * server sent, or `initial` before the first frame. At runtime it is a plain function, so it composes like any getter.
 */
export interface Replicated<T> extends Readable<T> {
	readonly [replicatedKind]: { readonly audience: "all"; readonly value: (value: T) => T };
}

/**
 * A value each player receives privately. On a client it reads the local player's value, or `initial` before the
 * first frame. On the server there is no single value, so calling it throws.
 */
export interface ReplicatedPerPlayer<T> extends Readable<T> {
	readonly [replicatedKind]: { readonly audience: "player"; readonly value: (value: T) => T };
}

const UNTRANSFORMED = "Crucible replicated state needs the @trioforge/crucible/transformer plugin";

/**
 * Declares a value every client receives. Write it as the initializer of an exported top-level const in a shared
 * module, with an explicit type argument and a literal name. The server writes it through `publish`; fields the type
 * does not declare are never sent.
 */
export function replicated<T = never>(_name: string, _initial: NoInfer<T>): Replicated<T> {
	error(UNTRANSFORMED);
}

/** Declares a value each player receives privately. The server supplies it with `publish(declaration, project)`. */
export function replicatedPerPlayer<T = never>(_name: string, _initial: NoInfer<T>): ReplicatedPerPlayer<T> {
	error(UNTRANSFORMED);
}

/**
 * The declaration's sync status. Inside an effect or a computed it tracks like a value. On the server, a per-player
 * declaration throws.
 */
export function syncStatus<T>(declaration: Replicated<T> | ReplicatedPerPlayer<T>): SyncStatus {
	return statusOf(declaration);
}
