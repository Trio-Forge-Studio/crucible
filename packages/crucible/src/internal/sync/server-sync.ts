import { ClientReady } from "../../network";
import { Subsystem } from "../../subsystem";
import { enginePort } from "../binding";
import { runtimeDiagnostic } from "../diagnostics";
import { stateOf } from "../lifecycle";
import { claim } from "../network/claims";
import { KICK_MESSAGE } from "../network/limits";
import { networkPort } from "../network/port";
import { FrameEvent } from "../port";
import { createComputedNode, currentEpoch, readComputed, type ComputedNode } from "../reactive/graph";
import {
	SYNC_HEARTBEAT_BYTES,
	SYNC_HEARTBEAT_PEERS,
	SYNC_HEARTBEAT_VISITS,
	SYNC_MAX_REVISION,
	SYNC_PEER_VISITS,
} from "../reactive/limits";
import { define, type TypeId } from "../registry";
import { declarations, useRetired, type Declaration } from "./declarations";
import { frameCost, overBudget, SyncFaultMessage, SyncFrameMessage, type SyncFrame, type SyncValue } from "./frame";
import { admit, equal, isOverBudget, spend, type Work } from "./values";

/** @internal How ServerSync sends: exactly `Network.send` on the frame message. */
export type FrameSender = (player: Player, frame: SyncFrame) => boolean;

/** What a peer accepted for one declaration: the version and frozen value of the last frame `send` took. */
interface Accepted {
	readonly version: number;
	readonly value: unknown;
}

/** A retired publication fails for good, at no version a projection can reach. */
const RETIRED = -1;

interface View {
	readonly player: Player;
	revision: number;
	live: boolean;
	readonly accepted: Map<Declaration, Accepted>;
	readonly projections: Map<Declaration, ComputedNode>;
	/** Declarations the client was told failed, with the version that failed or RETIRED. */
	readonly failing: Map<Declaration, number>;
	readonly reported: Set<Declaration>;
}

/** One examination, touching nothing on the peer until it commits (rule 58). */
interface Staged {
	readonly values: SyncValue[];
	readonly failed: string[];
	readonly accept: Map<Declaration, Accepted>;
	readonly failing: Map<Declaration, number>;
	readonly recovered: Declaration[];
	readonly reasons: Map<Declaration, string>;
}

let sender: FrameSender = () => false;

class ServerSync extends Subsystem {
	private readonly peers = new Map<Player, View>();
	private readonly order = new Array<View>();
	private cursor = 0;
	private settledEpoch = -1;
	private incomplete = false;
	private retiredSince = false;
	private examining = "";
	/** Visits the last Heartbeat spent, for specs of rule 70's bound. */
	spent = 0;
	private disconnect?: () => void;

	readonly ready = claim(this, "ready", ClientReady, (player: Player) => this.snapshot(player));
	readonly fault = claim(this, "fault", SyncFaultMessage, (player: Player) => this.faulted(player));

	constructor() {
		super();
		useRetired((declaration) => this.retired(declaration));
	}

	protected awake() {
		this.disconnect = enginePort().onFrame(FrameEvent.Heartbeat, () => this.heartbeat());
		const state = stateOf(this)!;
		if (state.subscriptions === undefined) state.subscriptions = [];
		state.subscriptions.push(() => this.stop());
	}

	private snapshot(player: Player): (() => void) | undefined {
		if (!declarations().some((declaration) => declaration.publication !== undefined)) return undefined;
		const view: View = {
			player,
			revision: 0,
			live: true,
			accepted: new Map(),
			projections: new Map(),
			failing: new Map(),
			reported: new Set(),
		};
		this.peers.set(player, view);
		this.order.push(view);
		for (const declaration of declarations()) {
			const publication = declaration.publication;
			if (declaration.audience !== "player" || publication === undefined || !publication.live) continue;
			const project = publication.project!;
			view.projections.set(
				declaration,
				createComputedNode(() => project(player)),
			);
		}
		const work: Work = { visits: 0, limit: SYNC_PEER_VISITS };
		const staged = this.examine(view, work);
		if (staged !== undefined) this.send(view, staged, "snapshot");
		return () => this.end(view);
	}

	private heartbeat() {
		const [ok, err] = pcall(() => this.pace());
		if (!ok) runtimeDiagnostic("error", `Crucible sync: internal error in the Heartbeat: ${tostring(err)}`);
	}

	/** Rules 58 and 70: round robin from the first peer the last Heartbeat did not complete. */
	private pace() {
		this.spent = 0;
		const ring = [...this.order];
		if (ring.size() === 0) return;
		if (!this.incomplete && !this.retiredSince && currentEpoch() === this.settledEpoch) return;
		const startEpoch = currentEpoch();
		this.retiredSince = false;
		let bytes = 0;
		let visits = 0;
		let examined = 0;
		let stopped: View | undefined;
		const first = this.cursor % ring.size();
		for (let i = 0; i < ring.size(); i++) {
			const view = ring[(first + i) % ring.size()];
			if (!view.live) continue;
			if (examined > 0 && (examined >= SYNC_HEARTBEAT_PEERS || visits >= SYNC_HEARTBEAT_VISITS)) {
				stopped = view;
				break;
			}
			const work: Work = { visits: 0, limit: SYNC_PEER_VISITS };
			const staged = this.examine(view, work);
			visits += work.visits;
			examined += 1;
			if (staged === undefined) continue;
			if (staged.values.isEmpty() && staged.failed.isEmpty()) {
				this.commit(view, staged, false);
				continue;
			}
			const result = this.send(view, staged, "update", bytes);
			if (result === undefined) {
				stopped = view;
				break;
			}
			bytes += result;
		}
		this.spent = visits;
		if (stopped !== undefined) {
			this.incomplete = true;
			this.cursor = math.max(this.order.indexOf(stopped), 0);
		} else {
			this.incomplete = false;
			this.cursor = 0;
			this.settledEpoch = startEpoch;
		}
	}

	/** One peer's staged examination, or undefined after failing the peer for passing its visit budget. */
	private examine(view: View, work: Work): Staged | undefined {
		const [ok, result] = pcall(() => this.stage(view, work));
		if (ok) return result;
		this.fail(
			view,
			isOverBudget(result)
				? `examination over SYNC_PEER_VISITS after ${work.visits} visits, at ${this.examining}`
				: `internal error at ${this.examining}: ${tostring(result)}`,
		);
		return undefined;
	}

	private stage(view: View, work: Work): Staged {
		const staged: Staged = {
			values: [],
			failed: [],
			accept: new Map(),
			failing: new Map(),
			recovered: [],
			reasons: new Map(),
		};
		for (const declaration of declarations()) {
			const publication = declaration.publication;
			if (publication === undefined) continue;
			this.examining = declaration.name;
			spend(work);
			const failedAt = view.failing.get(declaration);
			if (!publication.live) {
				if (failedAt === undefined) {
					staged.failed.push(declaration.name);
					staged.failing.set(declaration, RETIRED);
				}
				continue;
			}
			const accepted = view.accepted.get(declaration);
			let version: number;
			let value: unknown;
			if (declaration.audience === "all") {
				version = declaration.node.version;
				value = declaration.node.value;
				if (accepted !== undefined && accepted.version === version) continue;
			} else {
				const node = view.projections.get(declaration)!;
				const [pulled, raw] = pcall(readComputed, node);
				version = node.version;
				if (failedAt === version) continue;
				if (failedAt === undefined && accepted !== undefined && accepted.version === version) continue;
				let reason: string | undefined;
				if (pulled) {
					const [admitted, result] = pcall(admit, declaration.plan, declaration.name, raw, work);
					if (admitted) value = result;
					else if (isOverBudget(result)) error(result);
					else reason = tostring(result);
				} else reason = tostring(raw);
				if (reason !== undefined) {
					if (failedAt === undefined) {
						staged.failed.push(declaration.name);
						staged.reasons.set(declaration, reason);
					}
					staged.failing.set(declaration, version);
					continue;
				}
				if (failedAt !== undefined) {
					staged.values.push({ key: declaration.name, value });
					staged.accept.set(declaration, { version, value });
					staged.recovered.push(declaration);
					continue;
				}
			}
			if (accepted !== undefined && equal(accepted.value, value, work))
				staged.accept.set(declaration, { version, value: accepted.value });
			else {
				staged.values.push({ key: declaration.name, value });
				staged.accept.set(declaration, { version, value });
			}
		}
		this.examining = "";
		return staged;
	}

	/**
	 * Preflights and sends one frame. Returns its bytes once `send` took it, 0 when the peer ended or failed, or
	 * undefined when the Heartbeat's byte budget defers it.
	 */
	private send(view: View, staged: Staged, kind: "snapshot" | "update", spent = 0): number | undefined {
		const names = staged.values.map((entry) => entry.key);
		if (view.revision >= SYNC_MAX_REVISION) {
			this.fail(view, `revision passes SYNC_MAX_REVISION (${names.join(", ")})`);
			return 0;
		}
		const frame: SyncFrame = {
			failed: staged.failed,
			kind,
			revision: view.revision + 1,
			values: staged.values,
		};
		const cost = frameCost(frame, SyncFrameMessage.index!);
		const budget = overBudget(cost);
		if (budget !== undefined) {
			this.fail(
				view,
				`frame over ${budget} (${cost.bytes} bytes, ${cost.entries} entries, ${cost.visits} visits; ${names.join(", ")})`,
			);
			return 0;
		}
		if (spent > 0 && spent + cost.bytes > SYNC_HEARTBEAT_BYTES) return undefined;
		const [ok, accepted] = pcall(sender, view.player, frame);
		if (!ok) {
			this.fail(view, `encode refused a preflighted frame (${cost.bytes} bytes; ${names.join(", ")})`);
			return 0;
		}
		if (!accepted) {
			this.end(view);
			return 0;
		}
		this.commit(view, staged, true);
		return cost.bytes;
	}

	private commit(view: View, staged: Staged, sent: boolean) {
		if (sent) view.revision += 1;
		for (const [declaration, accepted] of staged.accept) view.accepted.set(declaration, accepted);
		for (const [declaration, version] of staged.failing) view.failing.set(declaration, version);
		for (const declaration of staged.recovered) view.failing.delete(declaration);
		for (const [declaration, reason] of staged.reasons) {
			if (view.reported.has(declaration)) continue;
			view.reported.add(declaration);
			runtimeDiagnostic("error", `Crucible sync: ${declaration.name} for ${view.player.Name} failed: ${reason}`);
		}
	}

	/** Rule 66: a failed frame best effort, the view ended, one diagnostic, then the kick. */
	private fail(view: View, reason: string) {
		if (!view.live) return;
		const player = view.player;
		pcall(sender, player, { failed: [], kind: "failed", revision: view.revision, values: [] });
		this.end(view);
		runtimeDiagnostic(
			"error",
			`Crucible sync for ${player.Name} failed at revision ${view.revision}: ${reason}; kicking`,
		);
		pcall(() => networkPort().kick(player, KICK_MESSAGE));
	}

	/** Rule 65, server side: only the sender's own view ends. */
	private faulted(player: Player) {
		const view = this.peers.get(player);
		if (view !== undefined && view.live) {
			this.end(view);
			runtimeDiagnostic(
				"error",
				`Crucible sync for ${player.Name} stopped on the client after revision ${view.revision}; kicking`,
			);
		}
		pcall(() => networkPort().kick(player, KICK_MESSAGE));
	}

	/** Idempotent. The peer's projections end with it. */
	private end(view: View) {
		if (!view.live) return;
		view.live = false;
		view.projections.clear();
		if (this.peers.get(view.player) === view) this.peers.delete(view.player);
		const index = this.order.indexOf(view);
		if (index !== -1) this.order.remove(index);
	}

	private retired(declaration: Declaration) {
		this.retiredSince = true;
		for (const view of this.order) view.projections.delete(declaration);
	}

	/** Rule 73: ServerSync's own generation ended. */
	private stop() {
		this.disconnect?.();
		this.disconnect = undefined;
		for (const view of [...this.order]) {
			pcall(sender, view.player, { failed: [], kind: "failed", revision: view.revision, values: [] });
			this.end(view);
		}
	}
}

let installed = false;

/** @internal Rule 56: the server entry defines ServerSync once per realm, constructed before every game Subsystem. */
export function installServerSync(send: FrameSender) {
	sender = send;
	if (installed) return;
	installed = true;
	define(ServerSync, { kind: "subsystem", id: "Crucible.ServerSync" as TypeId, deps: [], provides: [] }, true);
}
