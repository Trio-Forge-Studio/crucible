import { runtimeDiagnostic } from "../diagnostics";
import { defer, within } from "../lifecycle";
import { CallResult, disown, own } from "../runner";
import { Hook } from "../state";
import type { Definition } from "./catalog";
import { type Claim, isLive } from "./claims";
import { controlPacket, encodeBody, ErrorReason, FailReason, frame, Opcode, type Packet } from "./envelope";
import * as L from "./limits";
import type { Cancel, NetworkPort } from "./port";

/** @internal Boot drives a session through these, in order, inside one transaction (rules 79, 80). */
export interface NetworkLifecycle {
	/** Before load. Server: peers and an unparented remote. Client: discovers and adopts the catalog, and may yield. */
	open(): void;
	/** After load. Server: seals the catalog. Sends are allowed from here on (rule 17). */
	loaded(): void;
	/** Boot's last fallible step: freezes claims, then the server publishes or the client fires READY and flushes. */
	commit(): void;
	/** A boot failure, or after the shutdown sweep. Idempotent and never throws. */
	close(): void;
}

/** @internal */
export const enum Stage {
	Idle,
	Open,
	Loaded,
	Committed,
	Closed,
}

/** @internal Server: whether this player's READY arrived. Client: whether READY was fired. */
export const enum PeerState {
	Waiting,
	Ready,
	Ended,
}

/** @internal An event or request callback that started and has not returned (rules 42, 45, 52 to 55). */
export interface Running {
	readonly claim: Claim;
	/** Server requests only: where the one reply goes. */
	readonly request?: { readonly definition: Definition; readonly nonce: number };
	done: boolean;
	/** Counts toward MAX_ACTIVE_HANDLERS until it returns or passes its lifetime (rules 54, 55). */
	counted: boolean;
	/** The reply was given up because the owner retired first (rule 29). */
	settled: boolean;
	/** This run already reported a failure or an overrun (rule 75). */
	reported: boolean;
	lifetime?: Cancel;
}

/** @internal A ClientReady cleanup kept for a peer (rule 35). */
export interface Cleanup {
	readonly claim: Claim;
	readonly cleanup: Callback;
}

/** @internal One player on the server; the server itself on the client. */
export interface Peer {
	readonly player?: Player;
	state: PeerState;
	readonly queue: Packet[];
	queuedBytes: number;
	readonly running: Running[];
	counted: number;
	/** Server: the highest admitted nonce (rule 41). */
	lastRequestNonce: number;
	/** Stored in ClientReady order; ending the peer defers them in reverse (rule 35). */
	readonly cleanups: Cleanup[];
	/** Server: message names already diagnosed for a rejected packet, plus the envelope sentinel (rule 74). */
	readonly rejected: Set<Definition | "envelope">;
	/** Server: receiver fields already diagnosed for this player while the peer lives (rule 75). */
	readonly failedClaims: Set<Claim>;
}

/** @internal */
export function newPeer(player?: Player): Peer {
	return {
		player,
		state: PeerState.Waiting,
		queue: [],
		queuedBytes: 0,
		running: [],
		counted: 0,
		lastRequestNonce: 0,
		cleanups: [],
		rejected: new Set(),
		failedClaims: new Set(),
	};
}

function traceback(err: unknown) {
	return debug.traceback(tostring(err), 2);
}

function invoke(callback: Callback, argument: unknown): unknown {
	return callback(argument);
}

// compiler-types 3.0.0-types.0 does not declare Promise.is; the RuntimeLib's TS.Promise has it.
function isPromise(value: unknown): boolean {
	return (Promise as unknown as { readonly is: (value: unknown) => boolean }).is(value);
}

/** @internal The protocol core both sides share. One session owns every piece of network state in its realm. */
export abstract class Session implements NetworkLifecycle {
	protected stage = Stage.Idle;
	/** Receiver fields diagnosed under the session-level key (rule 75). Bounded by the receiver count. */
	protected readonly lateFailures = new Set<Claim>();
	/** Messages diagnosed for an event with no live receiver (rule 56). */
	protected readonly noReceiver = new Set<Definition>();

	constructor(protected readonly port: NetworkPort) {}

	abstract open(): void;
	abstract loaded(): void;
	abstract commit(): void;
	abstract close(): void;

	protected abstract fire(peer: Peer, packet: Packet): void;
	/** Terminal failure (rule 77). */
	protected abstract fail(peer: Peer, reason: FailReason): void;

	/** Rule 17: throws before boot loaded its modules; false once the session closed. */
	protected sendable(): boolean {
		if (this.stage === Stage.Idle || this.stage === Stage.Open)
			error("Crucible networking is not ready; send from a constructor, a hook, or later", 0);
		return this.stage !== Stage.Closed;
	}

	/** A ready peer fires now; a waiting one queues within its bounds or fails (rules 20, 22, 50). */
	protected deliver(peer: Peer, packet: Packet): boolean {
		if (peer.state === PeerState.Ready) {
			this.fire(peer, packet);
			return true;
		}
		if (peer.state === PeerState.Ended) return false;
		if (peer.queue.size() >= L.MAX_QUEUED_PACKETS || packet.charge > L.MAX_QUEUED_BYTES - peer.queuedBytes) {
			this.fail(peer, FailReason.Overflow);
			return false;
		}
		peer.queue.push(packet);
		peer.queuedBytes += packet.charge;
		return true;
	}

	/** Fires a peer's queue in send order, after it became ready (rule 46). */
	protected flush(peer: Peer) {
		const queue = [...peer.queue];
		peer.queue.clear();
		peer.queuedBytes = 0;
		for (const packet of queue) this.fire(peer, packet);
	}

	/**
	 * Starts an event or request callback for a decoded packet on a thread its Subsystem owns (rule 52). Runs from the
	 * remote signal, never inside a drain, to its first yield before returning.
	 */
	protected start(peer: Peer, claim: Claim, args: readonly unknown[], request?: Running["request"]) {
		const entry: Running = { claim, request, done: false, counted: true, settled: false, reported: false };
		peer.running.push(entry);
		peer.counted += 1;
		const thread = coroutine.create(() => {
			const [ok, value] = xpcall(claim.callback, traceback, ...args);
			disown(coroutine.running());
			this.finish(peer, entry, ok, value);
		});
		own(thread, claim.owner);
		this.port.resume(thread);
		if (!entry.done && peer.state !== PeerState.Ended)
			entry.lifetime = this.port.delay(L.HANDLER_LIFETIME_SECONDS, () => this.overrun(peer, entry));
	}

	private uncount(peer: Peer, entry: Running) {
		if (!entry.counted) return;
		entry.counted = false;
		peer.counted -= 1;
	}

	private overrun(peer: Peer, entry: Running) {
		entry.lifetime = undefined;
		if (entry.done) return;
		this.reportCallback(peer, entry.claim, entry, `still running after ${L.HANDLER_LIFETIME_SECONDS} seconds`);
		this.uncount(peer, entry);
	}

	/** A callback returned or threw (rules 42, 45, 53). */
	private finish(peer: Peer, entry: Running, ok: boolean, value: unknown) {
		entry.done = true;
		entry.lifetime?.();
		entry.lifetime = undefined;
		const index = peer.running.indexOf(entry);
		if (index !== -1) peer.running.remove(index);
		this.uncount(peer, entry);
		let failure: string | undefined;
		const request = entry.request;
		if (!ok) failure = tostring(value);
		else if (request === undefined) {
			if (value !== undefined) failure = `returned a ${typeOf(value)}; event callbacks return nothing`;
		} else if (isPromise(value)) failure = "returned a Promise; request callbacks return the response";
		const replies = request !== undefined && !entry.settled && peer.state !== PeerState.Ended;
		let packet: Packet | undefined;
		if (replies && failure === undefined) {
			const { definition, nonce } = request;
			const [encoded, result] = pcall(() =>
				frame(encodeBody(definition.name, definition.output!, "response", value), Opcode.Ok, [
					definition.index!,
					nonce,
				]),
			);
			if (encoded) packet = result;
			else failure = tostring(result);
		}
		if (failure !== undefined) this.reportCallback(peer, entry.claim, entry, failure);
		if (!replies) return;
		entry.settled = true;
		this.fire(
			peer,
			packet ?? controlPacket(Opcode.Error, [request.definition.index!, request.nonce], ErrorReason.HandlerError),
		);
	}

	/**
	 * Rule 75. One report per callback run. On the server, once per player per field while the peer lives, then once
	 * per field per session; on the client, always once per field per session.
	 */
	protected reportCallback(peer: Peer, claim: Claim, entry: Running | undefined, message: string) {
		if (entry !== undefined) {
			if (entry.reported) return;
			entry.reported = true;
		}
		const player = peer.player;
		const live = player !== undefined && peer.state !== PeerState.Ended;
		const seen = live ? peer.failedClaims : this.lateFailures;
		if (seen.has(claim)) return;
		seen.add(claim);
		runtimeDiagnostic("error", `${claim.label}${live ? ` for ${player.Name}` : ""} failed: ${message}`);
	}

	/** Runs a ClientReady callback or cleanup through lifecycle's `within`, which cancels a yield (rule 34). */
	protected runReady(
		peer: Peer,
		claim: Claim,
		work: Callback,
		argument: unknown,
	): LuaTuple<[ok: boolean, value: unknown]> {
		const [result, value] = within(claim.owner, undefined, Hook.Callback, invoke, work, argument);
		if (result === CallResult.Ok) return $tuple(true, value);
		this.reportCallback(peer, claim, undefined, tostring(value));
		return $tuple(false, undefined);
	}

	/** Never inline: a caller can be inside a drain, such as a retirement from lifecycle's release (rule 35). */
	protected deferCleanup(peer: Peer, entry: Cleanup) {
		defer(() => this.runReady(peer, entry.claim, entry.cleanup, undefined));
	}

	/** A ClientReady callback returned a cleanup. Kept only while its owner and its peer both live (rule 35). */
	protected keep(peer: Peer, entry: Cleanup) {
		if (isLive(entry.claim) && peer.state === PeerState.Ready) peer.cleanups.push(entry);
		else this.deferCleanup(peer, entry);
	}

	/**
	 * Rule 76. Idempotent: releases the queue, cancels lifetime timers, releases the diagnostic keys and defers the
	 * stored cleanups in reverse ClientReady order. Cancels no event or request callback (rule 45).
	 */
	protected end(peer: Peer): boolean {
		if (peer.state === PeerState.Ended) return false;
		peer.state = PeerState.Ended;
		peer.queue.clear();
		peer.queuedBytes = 0;
		for (const entry of peer.running) {
			entry.lifetime?.();
			entry.lifetime = undefined;
		}
		peer.running.clear();
		peer.rejected.clear();
		peer.failedClaims.clear();
		const cleanups = [...peer.cleanups];
		peer.cleanups.clear();
		for (let i = cleanups.size() - 1; i >= 0; i--) this.deferCleanup(peer, cleanups[i]);
		return true;
	}

	/** Rule 56: an event with no live receiver is dropped, diagnosed once per message per session. */
	protected unreceived(definition: Definition) {
		if (this.noReceiver.has(definition)) return;
		this.noReceiver.add(definition);
		runtimeDiagnostic("warning", `Crucible dropped ${definition.name}: it has no live receiver`);
	}
}
