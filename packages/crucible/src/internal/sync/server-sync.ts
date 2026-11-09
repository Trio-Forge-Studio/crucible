import { effect, getActiveSub, ReactiveFlags, setActiveSub, type ReactiveNode } from "@rbxts/charm";
import { ClientReady } from "../../network";
import { Subsystem } from "../../subsystem";
import { enginePort } from "../binding";
import { runtimeDiagnostic } from "../diagnostics";
import { stateOf } from "../lifecycle";
import { claim } from "../network/claims";
import type { Packet } from "../network/envelope";
import { KICK_MESSAGE } from "../network/limits";
import { networkPort } from "../network/port";
import { FrameEvent } from "../port";
import { checkCharm } from "../reactive/charm";
import { peek } from "../reactive/store";
import { call, CallResult } from "../runner";
import {
	SYNC_HEARTBEAT_BYTES,
	SYNC_HEARTBEAT_PEERS,
	SYNC_HEARTBEAT_SCAN,
	SYNC_HEARTBEAT_VISITS,
	SYNC_MAX_REVISION,
	SYNC_PEER_VISITS,
} from "../reactive/limits";
import { define, type TypeId } from "../registry";
import { declarationNamed, declarations, useChanged, useRetired, type Declaration } from "./declarations";
import {
	frameCost,
	overBudget,
	SyncFaultMessage,
	SyncFrameMessage,
	type FrameCost,
	type SyncFrame,
	type SyncValue,
} from "./frame";
import { beginPatches, difference, smaller } from "./patches";
import { admit, isOverBudget, spend, type Work } from "./values";

checkCharm();

/** @internal Delivers the frame's preflighted transport packet. */
export type FrameSender = (player: Player, frame: SyncFrame, packet: Packet) => boolean;

/** What a peer accepted for one declaration: the version and frozen value of the last frame `send` took. */
interface Accepted {
	readonly version: number;
	readonly value: unknown;
}

/** A retired publication fails for good, at no version a projection can reach. */
const RETIRED = -1;

/**
 * One player's projection of a per-player declaration, pulled when a Heartbeat or a snapshot examines it. A pull runs
 * the projector on a worker inside a Charm effect and moves `version` when the outcome changes: a value that is not
 * `rawequal` to the last one, or a different failure. A throw or a yield is the failure. A write that reaches what the
 * pull read only marks it stale, so projectors run within the Heartbeat's budgets and never in the code that wrote.
 */
interface Projection {
	readonly project: (player: Player) => unknown;
	version: number;
	ok: boolean;
	/** The value, or the failure with its traceback. */
	value: unknown;
	dirty: boolean;
	/** The effect that watches what the last pull read, and what stops it. */
	node?: ReactiveNode;
	stop?: () => void;
}

/**
 * Charm flags the effect while it propagates the write, before any effect runs. An earlier effect that throws ends the
 * flush and skips the effect's run, so the flag can be the only mark.
 */
function skipped(projection: Projection): boolean {
	const node = projection.node;
	return node !== undefined && bit32.btest(node.flags, ReactiveFlags.Dirty | ReactiveFlags.Pending);
}

function pull(projection: Projection, player: Player, changed: () => void) {
	if (!projection.dirty && !skipped(projection)) return;
	projection.dirty = false;
	projection.stop?.();
	let pulling = true;
	const active = setActiveSub(undefined);
	projection.stop = effect(() => {
		if (!pulling) {
			// Reads nothing, so Charm drops what the pull read until the next pull.
			projection.dirty = true;
			changed();
			return;
		}
		pulling = false;
		projection.node = getActiveSub();
		const [result, value] = call(projection.project, player, undefined);
		const ok = result === CallResult.Ok;
		if (projection.version > 0 && ok === projection.ok && rawequal(value, projection.value)) return;
		projection.ok = ok;
		projection.value = value;
		projection.version += 1;
	});
	setActiveSub(active);
}

interface View {
	readonly player: Player;
	revision: number;
	live: boolean;
	readonly accepted: Map<Declaration, Accepted>;
	readonly projections: Map<Declaration, Projection>;
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

function sendFailed(player: Player, revision: number) {
	const frame: SyncFrame = { failed: [], kind: "failed", revision, values: [] };
	return sender(player, frame, frameCost(frame, SyncFrameMessage.index!).packet);
}

class ServerSync extends Subsystem {
	private readonly peers = new Map<Player, View>();
	private readonly order = new Array<View>();
	private cursor = 0;
	/** The peer the next quiet Heartbeat's scan starts at. */
	private scanned = 0;
	/** A published value or a projection changed since the last Heartbeat began a round. */
	private changed = true;
	private incomplete = false;
	private retiredSince = false;
	private examining = "";
	/** Visits the last Heartbeat spent, its scan's included, for specs of rule 70's bound. */
	spent = 0;
	/** Peers the last Heartbeat examined, for specs of rule 58. */
	examined = 0;
	private disconnect?: () => void;

	readonly ready = claim(this, "ready", ClientReady, (player: Player) => this.snapshot(player));
	readonly fault = claim(this, "fault", SyncFaultMessage, (player: Player) => this.faulted(player));

	constructor() {
		super();
		useRetired((declaration) => this.retired(declaration));
		useChanged(this.marker);
	}

	private readonly marker = () => {
		this.changed = true;
	};

	protected awake() {
		this.disconnect = enginePort().onFrame(FrameEvent.Heartbeat, () => this.heartbeat());
		const state = stateOf(this)!;
		if (state.subscriptions === undefined) state.subscriptions = [];
		state.subscriptions.push(() => this.stop());
	}

	private snapshot(player: Player): (() => void) | undefined {
		beginPatches();
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
			view.projections.set(declaration, {
				project: publication.project!,
				version: 0,
				ok: true,
				value: undefined,
				dirty: true,
			});
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

	/**
	 * Rules 58 and 70: round robin from the first peer the last Heartbeat did not complete. A Heartbeat with nothing to
	 * send scans instead, and its visits count against the round that a skipped projection starts.
	 */
	private pace() {
		beginPatches();
		this.spent = 0;
		this.examined = 0;
		if (this.order.size() === 0) return;
		let visits = 0;
		if (!this.incomplete && !this.retiredSince && !this.changed) {
			visits = this.scan();
			if (!this.changed) {
				this.spent = visits;
				return;
			}
		}
		this.changed = false;
		this.retiredSince = false;
		const ring = [...this.order];
		let bytes = 0;
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
		this.examined = examined;
		if (stopped !== undefined) {
			this.incomplete = true;
			this.cursor = math.max(this.order.indexOf(stopped), 0);
		} else {
			this.incomplete = false;
			this.cursor = 0;
		}
	}

	/**
	 * Looks for a projection whose effect a failed flush skipped, which only its flags show. It reads whole peers'
	 * projections, round robin from where the last scan stopped, and counts a visit for each peer and each projection.
	 * SYNC_HEARTBEAT_SCAN is a threshold checked before each peer, so a scan stops after the peer that reaches it, or
	 * after every peer. A sweep over every peer takes ceil(peers / k) quiet Heartbeats, where k is
	 * ceil(SYNC_HEARTBEAT_SCAN / (projections per peer + 1)). The round it starts pulls the projection when it reaches
	 * that peer, within rule 58's limits.
	 */
	private scan(): number {
		const size = this.order.size();
		let visits = 0;
		for (let peers = 0; peers < size && visits < SYNC_HEARTBEAT_SCAN; peers++) {
			if (this.scanned >= size) this.scanned = 0;
			const view = this.order[this.scanned];
			this.scanned += 1;
			visits += 1;
			for (const [, projection] of view.projections) {
				visits += 1;
				if (!projection.dirty && skipped(projection)) this.changed = true;
			}
			if (this.changed) break;
		}
		return visits;
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
				version = declaration.version;
				value = peek(declaration.node);
				if (accepted !== undefined && accepted.version === version) continue;
			} else {
				const projection = view.projections.get(declaration)!;
				pull(projection, view.player, this.marker);
				const pulled = projection.ok;
				const raw = projection.value;
				version = projection.version;
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
					staged.values.push({ key: declaration.name, mode: "full", value });
					staged.accept.set(declaration, { version, value });
					staged.recovered.push(declaration);
					continue;
				}
			}
			if (accepted !== undefined && difference(accepted.value, value, work) === undefined)
				staged.accept.set(declaration, { version, value: accepted.value });
			else {
				staged.values.push({ key: declaration.name, mode: "full", value });
				staged.accept.set(declaration, { version, value });
			}
		}
		for (const entry of staged.values) {
			this.examining = entry.key;
			const declaration = declarationNamed(entry.key)!;
			const accepted = view.accepted.get(declaration);
			const before = accepted !== undefined ? accepted.value : declaration.initial;
			if (view.revision !== 0) {
				const delta = difference(before, entry.value, work);
				if (delta === undefined) {
					entry.mode = "same";
					entry.value = undefined;
				} else if (smaller(delta, entry.value)) {
					entry.mode = "patch";
					entry.value = delta;
				}
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
		let [encoded, result] = pcall(frameCost, frame, SyncFrameMessage.index!);
		if (!encoded || overBudget(result as FrameCost) !== undefined) {
			let replaced = false;
			for (const entry of staged.values) {
				if (entry.mode !== "patch") continue;
				entry.mode = "full";
				entry.value = staged.accept.get(declarationNamed(entry.key)!)!.value;
				replaced = true;
			}
			if (replaced) [encoded, result] = pcall(frameCost, frame, SyncFrameMessage.index!);
		}
		if (!encoded) {
			this.fail(view, `frame encode failed (${names.join(", ")}): ${tostring(result)}`);
			return 0;
		}
		const cost = result as FrameCost;
		const budget = overBudget(cost);
		if (budget !== undefined) {
			this.fail(
				view,
				`frame over ${budget} (${cost.bytes} bytes, ${cost.entries} entries, ${cost.visits} visits; ${names.join(", ")})`,
			);
			return 0;
		}
		if (spent > 0 && spent + cost.bytes > SYNC_HEARTBEAT_BYTES) return undefined;
		const [ok, accepted] = pcall(sender, view.player, frame, cost.packet);
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
		pcall(sendFailed, player, view.revision);
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
		for (const [, projection] of view.projections) projection.stop?.();
		view.projections.clear();
		if (this.peers.get(view.player) === view) this.peers.delete(view.player);
		const index = this.order.indexOf(view);
		if (index === -1) return;
		this.order.remove(index);
		if (index < this.scanned) this.scanned -= 1;
	}

	private retired(declaration: Declaration) {
		this.retiredSince = true;
		for (const view of this.order) {
			view.projections.get(declaration)?.stop?.();
			view.projections.delete(declaration);
		}
	}

	/** Rule 73: ServerSync's own generation ended. */
	private stop() {
		this.disconnect?.();
		this.disconnect = undefined;
		for (const view of [...this.order]) {
			pcall(sendFailed, view.player, view.revision);
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
