import { defineFramework, type Definition } from "../network/catalog";
import { n } from "../network/schema";
import { SYNC_FRAME_BYTES, SYNC_FRAME_COLLECTION_ENTRIES, SYNC_FRAME_VALUE_VISITS } from "../reactive/limits";
import { costOf } from "./values";

/** @internal One declaration's whole value in a frame. A nil `value` is a real root `undefined`. */
export interface SyncValue {
	readonly key: string;
	readonly value: unknown;
}

/** @internal One atomic observation unit for one peer. Both lists are strictly ascending by key. */
export interface SyncFrame {
	readonly failed: readonly string[];
	readonly kind: "failed" | "snapshot" | "update";
	readonly revision: number;
	readonly values: readonly SyncValue[];
}

/** @internal */
export const SyncFrameMessage: Definition = defineFramework({
	kind: "to-client",
	name: "Crucible.State.Frame",
	input: n.object([
		["failed", n.array(n.string)],
		["kind", n.union([n.literal("failed"), n.literal("snapshot"), n.literal("update")])],
		["revision", n.u32],
		[
			"values",
			n.array(
				n.object([
					["key", n.string],
					["value", n.dynamic],
				]),
			),
		],
	]),
});

/** @internal The last revision a client applied before its sync stopped. */
export const SyncFaultMessage: Definition = defineFramework({
	kind: "to-server",
	name: "Crucible.State.Fault",
	input: n.u32,
});

function varuintSize(value: number): number {
	let size = 1;
	while (value >= 128) {
		value = math.floor(value / 128);
		size += 1;
	}
	return size;
}

function stringBytes(text: string): number {
	return varuintSize(text.size()) + text.size();
}

/** @internal The whole EVENT packet `send` encodes for `frame`, with visits and entries as the decoder counts them. */
export interface FrameCost {
	readonly bytes: number;
	readonly entries: number;
	readonly visits: number;
}

/**
 * @internal
 * Rule 69: the envelope (opcode, catalog index, slot count 0), then the frame object, `failed`, the kind's union
 * selection and literal, the u32 revision, and each value entry with its value's cached admission cost.
 */
export function frameCost(frame: SyncFrame, catalogIndex: number): FrameCost {
	let bytes = 1 + varuintSize(catalogIndex) + 1;
	let entries = frame.failed.size() + frame.values.size();
	let visits = 1;
	visits += 1;
	bytes += varuintSize(frame.failed.size());
	for (const key of frame.failed) {
		visits += 1;
		bytes += stringBytes(key);
	}
	visits += 2;
	bytes += 1;
	visits += 1;
	bytes += 4;
	visits += 1;
	bytes += varuintSize(frame.values.size());
	for (const entry of frame.values) {
		visits += 2;
		bytes += stringBytes(entry.key);
		const cost = costOf(entry.value);
		bytes += cost.bytes;
		entries += cost.entries;
		visits += cost.visits;
	}
	return { bytes, entries, visits };
}

/** @internal The first frame budget `cost` passes, or undefined when it fits. */
export function overBudget(cost: FrameCost): string | undefined {
	if (cost.bytes > SYNC_FRAME_BYTES) return "SYNC_FRAME_BYTES";
	if (cost.entries > SYNC_FRAME_COLLECTION_ENTRIES) return "SYNC_FRAME_COLLECTION_ENTRIES";
	if (cost.visits > SYNC_FRAME_VALUE_VISITS) return "SYNC_FRAME_VALUE_VISITS";
	return undefined;
}
