import type { ObjectState } from "./state";
const claims = new Map<thread, ObjectState>();
/** @internal */
export function setClaim(state: ObjectState) {
	claims.set(coroutine.running(), state);
}
/** @internal */
export function readClaim(): ObjectState | undefined {
	return claims.get(coroutine.running());
}
/** @internal */
export function clearClaim(thread = coroutine.running()) {
	claims.delete(thread);
}
