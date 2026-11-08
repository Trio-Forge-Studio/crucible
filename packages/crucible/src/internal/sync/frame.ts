import { defineFramework, type Definition } from "../network/catalog";
import { n } from "../network/schema";
import { SYNC_FRAME_BYTES, SYNC_FRAME_COLLECTION_ENTRIES, SYNC_FRAME_VALUE_VISITS } from "../reactive/limits";
import { encodeBody, frame as packet } from "../network/envelope";

/** @internal One declaration's full value, Charm Sync delta, or status-only recovery. */
export interface SyncValue {
	readonly key: string;
	mode: "full" | "patch" | "same";
	value: unknown;
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
					["mode", n.union([n.literal("full"), n.literal("patch"), n.literal("same")])],
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

/** @internal */
export interface FrameCost {
	readonly bytes: number;
	readonly entries: number;
	readonly visits: number;
}

/** @internal Preflight with the transport codec, including its envelope and decoder budgets. */
export function frameCost(frame: SyncFrame, catalogIndex: number): FrameCost {
	const body = encodeBody(SyncFrameMessage.name, SyncFrameMessage.input!, "payload", frame);
	const encoded = packet(body, 0, [catalogIndex]);
	return {
		bytes: buffer.len(encoded.bytes),
		entries: body.writer.budget.entries,
		visits: body.writer.budget.visits,
	};
}

/** @internal The first frame budget `cost` passes, or undefined when it fits. */
export function overBudget(cost: FrameCost): string | undefined {
	if (cost.bytes > SYNC_FRAME_BYTES) return "SYNC_FRAME_BYTES";
	if (cost.entries > SYNC_FRAME_COLLECTION_ENTRIES) return "SYNC_FRAME_COLLECTION_ENTRIES";
	if (cost.visits > SYNC_FRAME_VALUE_VISITS) return "SYNC_FRAME_VALUE_VISITS";
	return undefined;
}
