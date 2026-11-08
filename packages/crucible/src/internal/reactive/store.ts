import { endBatch, setActiveSub, startBatch } from "@rbxts/charm";
import { runtimeDiagnostic } from "../diagnostics";

/** @internal A value read without tracking it. */
export function peek<T>(read: () => T): T {
	const active = setActiveSub(undefined);
	const value = read();
	setActiveSub(active);
	return value;
}

/** @internal Ends a batch the caller opened. An effect that throws then is reported after `what`, not rethrown. */
export function settle(what: string) {
	const [ok, err] = pcall(endBatch);
	if (!ok) runtimeDiagnostic("error", `Crucible: an effect failed after ${what}: ${tostring(err)}`);
}

/** @internal An atom of any value type, as `store` writes it. */
export type Target = (value: never) => unknown;

/** @internal One atom and the value `store` gives it. */
export type Write = readonly [target: Target, value: unknown];

/**
 * @internal
 * Writes atoms Crucible owns in one batch, so their readers see every value at once and no effect runs inside
 * Crucible's own bookkeeping. None of the values is a function, which an atom would call.
 */
export function store(writes: readonly Write[], what: string) {
	startBatch();
	for (const [target, value] of writes) (target as (value: unknown) => unknown)(value);
	settle(what);
}
