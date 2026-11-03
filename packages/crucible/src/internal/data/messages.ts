import type { Node } from "../network/schema";
import { admit } from "./admission";
import type { MessageDecl } from "./inbox";
import { MESSAGE_NAME_MAX_BYTES } from "./limits";
import { dataRegistrySealed } from "./record-declarations";

const byName = new Map<string, MessageDecl>();
const byToken = setmetatable(new Map<object, MessageDecl>(), { __mode: "k" });

/**
 * @internal The emitted `profileMessage` (rules 2, 3, 65): the name re-checked, the payload admitted by rule 76 against
 * its descriptor, so a newer sender's added optional field stays in place unread. Shared-safe.
 */
export function declareProfileMessage(name: string, payload: Node): object {
	if (name.size() === 0 || name.size() > MESSAGE_NAME_MAX_BYTES)
		error(`${name}: a profile message name is 1 to ${MESSAGE_NAME_MAX_BYTES} bytes`, 3);
	const segments = name.split(".");
	for (const segment of segments)
		if (segment.match("^[A-Za-z][A-Za-z0-9_]*$")[0] === undefined)
			error(`${name}: a profile message name is dot-separated segments of letters, digits and underscores`, 3);
	if (segments[0] === "Crucible") error(`${name}: the name Crucible is reserved`, 3);
	if (dataRegistrySealed())
		error(`${name} was declared after the data registry sealed; declare messages in modules that boot loads`, 3);
	if (byName.has(name)) error(`${name} is already declared`, 3);
	const declaration: MessageDecl = {
		name,
		admit: (value) => {
			const result = admit(payload, value);
			return result.ok ? { ok: true, value: result.value } : { ok: false };
		},
	};
	const token = table.freeze({});
	byName.set(name, declaration);
	byToken.set(token, declaration);
	return token;
}

/** @internal The declaration behind a `ProfileMessage` value, or undefined for anything else. */
export function messageOf(token: unknown): MessageDecl | undefined {
	return typeIs(token, "table") ? byToken.get(token) : undefined;
}
