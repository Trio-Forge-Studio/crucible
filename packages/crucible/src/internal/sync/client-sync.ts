import { Subsystem } from "../../subsystem";
import { runtimeDiagnostic } from "../diagnostics";
import { claim } from "../network/claims";
import { commitPrepared, type PreparedWrite } from "../reactive/graph";
import { define, type TypeId } from "../registry";
import { declarationNamed, declarations } from "./declarations";
import { SyncFrameMessage, type SyncFrame } from "./frame";
import { reuse } from "./values";

/** @internal How ClientSync reports its last applied revision: exactly `Network.send` on the fault message. */
export type FaultSender = (revision: number) => void;

let sendFault: FaultSender = () => {};

function ascending(keys: readonly string[]): boolean {
	for (let i = 1; i < keys.size(); i++) if (!(keys[i - 1] < keys[i])) return false;
	return true;
}

function disjoint(values: readonly string[], failed: readonly string[]): boolean {
	let i = 0;
	let j = 0;
	while (i < values.size() && j < failed.size()) {
		if (values[i] === failed[j]) return false;
		if (values[i] < failed[j]) i += 1;
		else j += 1;
	}
	return true;
}

class ClientSync extends Subsystem {
	private revision = 0;
	private snapshotted = false;
	private failed = false;

	readonly frames = claim(this, "frames", SyncFrameMessage, (frame: SyncFrame) => this.apply(frame));

	/** Rules 62 to 65. A network callback thread, outside any drain; never yields and never throws. */
	private apply(frame: SyncFrame) {
		if (this.failed) return;
		if (frame.kind === "failed")
			return this.latch(`the server stopped this client's sync at revision ${frame.revision}`, false);
		const keys = frame.values.map((entry) => entry.key);
		if (frame.kind === "snapshot" ? this.snapshotted || frame.revision !== 1 : !this.snapshotted)
			return this.latch(`unexpected ${frame.kind} at revision ${frame.revision} after ${this.revision}`, true);
		if (frame.kind === "update" && frame.revision !== this.revision + 1)
			return this.latch(`revision ${frame.revision} does not follow ${this.revision}`, true);
		if (!ascending(keys) || !ascending(frame.failed) || !disjoint(keys, frame.failed))
			return this.latch(`revision ${frame.revision} lists keys out of order or twice`, true);
		const [prepared, writes] = pcall(() => {
			const out = new Array<PreparedWrite>();
			for (const entry of frame.values) {
				const declaration = declarationNamed(entry.key);
				if (declaration === undefined) continue;
				const reused = reuse(declaration.node.value, entry.value);
				if (!rawequal(reused, declaration.node.value)) out.push([declaration.node, reused]);
				out.push([declaration.status, "ready"]);
			}
			for (const key of frame.failed) {
				const declaration = declarationNamed(key);
				if (declaration !== undefined) out.push([declaration.status, "failed"]);
			}
			return out;
		});
		if (!prepared)
			return this.latch(`internal error while preparing revision ${frame.revision}: ${tostring(writes)}`, true);
		commitPrepared(writes);
		this.snapshotted = true;
		this.revision = frame.revision;
	}

	/** Rule 65: every value and the revision kept, every status "failed", the fault once, one diagnostic. */
	private latch(reason: string, fault: boolean) {
		this.failed = true;
		const [, err] = pcall(() =>
			commitPrepared(declarations().map((declaration): PreparedWrite => [declaration.status, "failed"])),
		);
		if (fault) pcall(sendFault, this.revision);
		runtimeDiagnostic(
			"error",
			`Crucible sync stopped: ${reason}${err === undefined ? "" : `; failing statuses: ${tostring(err)}`}`,
		);
	}
}

let installed = false;

/** @internal Rule 56: the client entry defines ClientSync before boot loads any game module, once per realm. */
export function installClientSync(fault: FaultSender) {
	sendFault = fault;
	if (installed) return;
	installed = true;
	define(ClientSync, { kind: "subsystem", id: "Crucible.ClientSync" as TypeId, deps: [], provides: [] });
}
