import { ClientReady, type Receiver } from "../../network";
import { readClaim } from "../construction-claim";
import type { ClassRecord } from "../registry";
import { Kind, type ObjectState } from "../state";
import { definitionOf, type Definition } from "./catalog";

/** @internal */
export interface Claim {
	readonly owner: ObjectState;
	readonly label: string;
	readonly target: Definition | ClientReady;
	readonly callback: Callback;
}

/** @internal */
export interface ClaimSession {
	readonly side: "server" | "client";
	readonly retired: (claim: Claim) => void;
}

let session: ClaimSession | undefined;
let frozen = false;
const receivers = new Map<Definition, Claim>();
const ready = new Array<Claim>();
const live = new Set<Claim>();

function retire(claim: Claim) {
	if (!live.delete(claim)) return;
	if (claim.target === ClientReady) ready.remove(ready.indexOf(claim));
	else receivers.delete(claim.target as Definition);
	session?.retired(claim);
}

/**
 * @internal
 * Re-checks what the transformer checked, because untransformed or cast code skips it. A violation throws from the
 * constructor and fails boot.
 */
export function claim(owner: object, field: string, target: unknown, callback: unknown): Receiver {
	const state = readClaim();
	if (state === undefined || state.object !== owner)
		error("receive() must initialize a readonly field of the Subsystem being constructed");
	const record = state.record as ClassRecord;
	const label = `${record.id}.${field}`;
	if (record.kind !== Kind.Subsystem)
		error(`${label}: receive() must initialize a readonly field of a Subsystem; behaviours do not receive`);
	if (frozen) error(`${label}: receive() after the network commit`);
	const side = session?.side ?? error(`${label}: receive() before the network session opened`);
	if (!typeIs(callback, "function")) error(`${label}: receive() needs a callback`);
	if (target === ClientReady) {
		if (side !== "server") error(`${label}: ClientReady is received in a server Subsystem.`);
		for (const other of ready)
			if (other.owner === state)
				error(`${label}: ${other.label} already receives ClientReady; a Subsystem holds one`);
	} else {
		const definition =
			definitionOf(target) ??
			error(`${label}: receive() needs a message declared with toServer, toClient or askServer`);
		const serverSide = definition.kind !== "to-client";
		if (serverSide !== (side === "server"))
			error(
				serverSide
					? `${label}: ${definition.name} is client-to-server; receive it in a server Subsystem.`
					: `${label}: ${definition.name} is server-to-client; receive it in a client Subsystem.`,
			);
		const other = receivers.get(definition);
		if (other !== undefined) error(`${definition.name} has two receivers: ${other.label} and ${label}`);
	}
	const entry: Claim = { owner: state, label, target: target as Definition | ClientReady, callback };
	if (target === ClientReady) ready.push(entry);
	else receivers.set(target as Definition, entry);
	live.add(entry);
	if (state.subscriptions === undefined) state.subscriptions = [];
	state.subscriptions.push(() => retire(entry));
	return table.freeze({}) as unknown as Receiver;
}

/** @internal */
export function isLive(claim: Claim): boolean {
	return live.has(claim);
}

/** @internal */
export function receiverOf(definition: Definition): Claim | undefined {
	return receivers.get(definition);
}

/** @internal */
export function readyClaims(): readonly Claim[] {
	return [...ready];
}

/** @internal */
export function openClaims(value: ClaimSession) {
	session = value;
}

/** @internal */
export function freezeClaims() {
	frozen = true;
}

/** @internal */
export function closeClaims(discard: boolean) {
	session = undefined;
	if (!discard) return;
	frozen = false;
	receivers.clear();
	ready.clear();
	live.clear();
}
