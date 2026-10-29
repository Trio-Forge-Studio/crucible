import type { ObjectState } from "./state";
let claim: ObjectState | undefined;
/** @internal */
export function setClaim(state: ObjectState) {
	claim = state;
}
/** @internal */
export function readClaim(): ObjectState | undefined {
	return claim;
}
/** @internal */
export function clearClaim() {
	claim = undefined;
}
