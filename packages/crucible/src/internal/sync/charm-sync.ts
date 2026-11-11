import { patch } from "@rbxts/charm-sync";

/** @internal The exact peer release covered by the sync specs. */
export const CHARM_SYNC_VERSION = "0.4.1";

// 0.4.1's declarations name applyImmutable, but the published Luau exports apply.
export const applyPatch = (
	typeIs(patch, "table") ? (patch as unknown as { readonly apply: unknown }).apply : undefined
) as (current: unknown, delta: unknown) => unknown;
let checked = false;

/** @internal Probe only when replicated declarations load, once per realm, without retaining any state. */
export function checkCharmSync() {
	if (checked) return;
	const [ok, result] = pcall(() => {
		if (!typeIs(applyPatch, "function") || !typeIs(patch.diff, "function")) return false;
		const removed = patch.diff(1, undefined) as { readonly __none?: string };
		if (!typeIs(removed, "table") || removed["__none"] !== "__none") return false;
		if (applyPatch(1, { __none: "__none" }) !== undefined) return false;
		const before = { nested: { keep: 1, drop: 2 }, stable: { value: 3 } };
		const after = { nested: { keep: 4 }, stable: before.stable };
		const delta = patch.diff(before, after);
		const value = applyPatch(before, delta) as typeof after;
		if (value === before || value.nested.keep !== 4 || (value.nested as { drop?: number }).drop !== undefined)
			return false;
		if (before.nested.keep !== 1 || before.nested.drop !== 2 || value.stable !== before.stable) return false;
		if (after.stable !== before.stable || (after.nested as { drop?: number }).drop !== undefined) return false;
		if (patch.diff(after, after) !== undefined || patch.diff(1, 2) !== 2) return false;
		if (patch.diff({ keep: 4 }, { keep: 4 }) !== undefined) return false;
		return applyPatch(undefined, 7) === 7;
	});
	if (!ok || result !== true)
		error(
			`Crucible needs @rbxts/charm-sync ${CHARM_SYNC_VERSION}; installed version unavailable (charm-sync exposes no runtime version), and its patch shape or behaviour differs. Install @rbxts/charm-sync ${CHARM_SYNC_VERSION} beside @rbxts/charm, then rebuild the game.`,
			0,
		);
	checked = true;
}
