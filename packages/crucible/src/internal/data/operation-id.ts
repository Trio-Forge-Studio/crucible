import { ID_EVICT_MIN_AGE_SECONDS, ID_FUTURE_SKEW_SECONDS } from "./limits";
import type { DataPort } from "./port";

const DIGITS = "0123456789abcdefghijklmnopqrstuvwxyz";

function base36(value: number): string {
	let rest = math.floor(value);
	let text = "";
	do {
		const digit = rest % 36;
		text = DIGITS.sub(digit + 1, digit + 1) + text;
		rest = math.floor(rest / 36);
	} while (rest > 0);
	return text;
}

/**
 * @internal An operation id (rules 69, 88): "o1-", the creation second in base 36, "-", and the platform's GUID without
 * its dashes. It carries its own creation time, so the age and floor checks need no stored record.
 */
export function newOperationId(port: DataPort): string {
	const [guid] = port.guid().lower().gsub("[^%w]", "");
	return `o1-${base36(port.wall())}-${guid}`;
}

/** @internal The creation second of an id of this format, and undefined for any other text. */
export function createdAt(id: string): number | undefined {
	const [seconds] = id.match("^o1%-([0-9a-z]+)%-[0-9a-z]+$");
	return seconds === undefined ? undefined : tonumber(seconds, 36);
}

/** @internal */
export type IdAge = "fresh" | "expired" | "future" | "malformed";

/** @internal Rule 69's horizon: at most `maxAgeSeconds` old and at most ID_FUTURE_SKEW_SECONDS ahead of `now`. */
export function ageOf(id: string, now: number, maxAgeSeconds: number): IdAge {
	const created = createdAt(id);
	if (created === undefined) return "malformed";
	if (created > now + ID_FUTURE_SKEW_SECONDS) return "future";
	if (now - created > maxAgeSeconds) return "expired";
	return "fresh";
}

/** @internal Whether a ring may evict this id at `now`: only once it is older than ID_EVICT_MIN_AGE_SECONDS (rule 69). */
export function evictable(id: string, now: number): boolean {
	const created = createdAt(id);
	return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS;
}
