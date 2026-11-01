import { runtimeDiagnostic } from "../diagnostics";
import { type Definition, definitionAt, definitionOf, discardIndices, seal } from "./catalog";
import { type Claim, closeClaims, freezeClaims, isLive, openClaims, readyClaims, receiverOf } from "./claims";
import {
	argumentCount,
	controlPacket,
	decodeBody,
	encodeBody,
	ErrorReason,
	failureReason,
	FailReason,
	frame,
	missingInstance,
	Opcode,
	openBody,
	type Packet,
	readHeader,
} from "./envelope";
import * as L from "./limits";
import type { Cancel } from "./port";
import { type Cleanup, newPeer, type Peer, PeerState, Session, Stage } from "./session";

const FAIL_NAMES = ["overflow", "overloaded", "closing", "malformed"];

/** @internal */
export class ServerSession extends Session {
	private readonly peers = new Map<Player, Peer>();
	private readonly order = new Array<Peer>();
	private readonly connections = new Array<Cancel>();
	private catalog = "";
	private published = false;
	private reportedNoPeer = false;

	open() {
		this.stage = Stage.Open;
		openClaims({ side: "server", retired: (claim) => this.retired(claim) });
		this.port.create((player, ...args) => this.receive(player, argumentCount(args), args[0], args[1]));
		this.connections.push(this.port.onPlayerAdded((player) => this.add(player)));
		this.connections.push(
			this.port.onPlayerRemoving((player) => {
				const peer = this.peers.get(player);
				if (peer !== undefined) this.end(peer);
			}),
		);
		for (const player of this.port.players()) this.add(player);
	}

	loaded() {
		this.catalog = seal();
		this.stage = Stage.Loaded;
	}

	commit() {
		freezeClaims();
		this.port.publish(L.PROTOCOL_VERSION, this.catalog);
		this.published = true;
		this.stage = Stage.Committed;
	}

	close() {
		if (this.stage === Stage.Closed) return;
		const committed = this.stage === Stage.Committed;
		this.stage = Stage.Closed;
		const closing = controlPacket(Opcode.Fail, [], FailReason.Closing);
		for (const peer of [...this.order]) {
			if (this.published) pcall(() => this.fire(peer, closing));
			pcall(() => this.end(peer));
		}
		for (const disconnect of this.connections) pcall(disconnect);
		this.connections.clear();
		pcall(() => this.port.destroy());
		closeClaims(!committed);
		if (!committed) discardIndices();
	}

	send(player: Player, token: unknown, value: unknown): boolean {
		const sendable = this.sendable();
		const definition = toClient(token, "Network.send");
		const body = encodeBody(definition.name, definition.input, "payload", value);
		if (!sendable) return false;
		const peer = this.peers.get(player);
		return peer !== undefined && this.deliver(peer, frame(body, Opcode.Event, [definition.index!]));
	}

	broadcast(token: unknown, value: unknown) {
		const sendable = this.sendable();
		const definition = toClient(token, "Network.broadcast");
		const body = encodeBody(definition.name, definition.input, "payload", value);
		if (!sendable) return;
		const packet = frame(body, Opcode.Event, [definition.index!]);
		for (const peer of [...this.order]) this.deliver(peer, packet);
	}

	protected fire(peer: Peer, packet: Packet) {
		this.port.fireClient(peer.player!, packet.bytes, packet.sidecar);
	}

	protected end(peer: Peer): boolean {
		if (peer.state === PeerState.Ended) return false;
		const player = peer.player!;
		if (this.peers.get(player) === peer) this.peers.delete(player);
		const index = this.order.indexOf(peer);
		if (index !== -1) this.order.remove(index);
		return super.end(peer);
	}

	/** Ends the peer before the kick: PlayerRemoving fires seconds after Kick returns, and packets keep arriving. */
	protected fail(peer: Peer, reason: FailReason) {
		if (peer.state === PeerState.Ended) return;
		const player = peer.player!;
		runtimeDiagnostic("error", `Crucible network peer ${player.Name} failed (${FAIL_NAMES[reason]}); kicking`);
		const packet = controlPacket(Opcode.Fail, [], reason);
		pcall(() => this.fire(peer, packet));
		this.end(peer);
		pcall(() => this.port.kick(player, L.KICK_MESSAGE));
	}

	private add(player: Player) {
		if (this.stage === Stage.Closed || this.peers.has(player)) return;
		const peer = newPeer(player);
		this.peers.set(player, peer);
		this.order.push(peer);
	}

	private retired(claim: Claim) {
		const cleanups = new Array<[Peer, Cleanup]>();
		for (const peer of this.order) {
			for (const entry of peer.running) {
				const request = entry.request;
				if (entry.claim !== claim || request === undefined || entry.settled) continue;
				entry.settled = true;
				this.fire(
					peer,
					controlPacket(Opcode.Error, [request.definition.index!, request.nonce], ErrorReason.Unavailable),
				);
			}
			for (let i = peer.cleanups.size() - 1; i >= 0; i--) {
				const entry = peer.cleanups[i];
				if (entry.claim !== claim) continue;
				peer.cleanups.remove(i);
				cleanups.push([peer, entry]);
			}
		}
		for (const [peer, entry] of cleanups) this.deferCleanup(peer, entry);
	}

	private reject(peer: Peer, key: Definition | "envelope", what: string, detail?: string) {
		if (peer.rejected.has(key)) return;
		peer.rejected.add(key);
		const reason = detail === undefined ? "" : `: ${detail}`;
		runtimeDiagnostic("warning", `Crucible dropped ${what} from ${peer.player!.Name}${reason}`);
	}

	private receive(player: Player, count: number, packet?: unknown, sidecar?: unknown) {
		const [ok, err] = pcall(() => this.dispatch(player, count, packet, sidecar));
		if (!ok) runtimeDiagnostic("error", `Crucible network receive failed: ${tostring(err)}`);
	}

	private dispatch(player: Player, count: number, packet: unknown, sidecar: unknown) {
		const peer = this.peers.get(player);
		if (peer === undefined) {
			if (this.reportedNoPeer) return;
			this.reportedNoPeer = true;
			runtimeDiagnostic("warning", "Crucible dropped a network packet from a player with no peer");
			return;
		}
		if (this.stage !== Stage.Committed) return this.reject(peer, "envelope", "a packet before the network commit");
		const header = readHeader(count, packet, sidecar);
		if (header === undefined) return this.reject(peer, "envelope", "a malformed network packet");
		if (header.opcode === Opcode.Ready) {
			if (peer.state !== PeerState.Waiting) return this.reject(peer, "envelope", "a second READY");
			return this.ready(peer);
		}
		if (header.opcode === Opcode.Fail) {
			const reason = FAIL_NAMES[header.reason] ?? `unknown reason ${header.reason}`;
			runtimeDiagnostic("error", `Crucible network peer ${player.Name} sent FAIL (${reason}); kicking`);
			this.end(peer);
			pcall(() => this.port.kick(player, L.KICK_MESSAGE));
			return;
		}
		if (header.opcode !== Opcode.Event && header.opcode !== Opcode.Request)
			return this.reject(peer, "envelope", "a server-to-client packet");
		if (peer.state !== PeerState.Ready) return this.reject(peer, "envelope", "a game packet before READY");
		const definition = definitionAt(header.index);
		if (definition === undefined) return this.reject(peer, "envelope", "an unknown message index");
		const event = header.opcode === Opcode.Event;
		if (definition.kind !== (event ? "to-server" : "ask-server"))
			return this.reject(peer, definition, `${definition.name} sent as the wrong kind of packet`);
		const [opened, reader] = pcall(openBody, packet as buffer, sidecar as Record<string, Instance>, header);
		if (!opened) return this.reject(peer, definition, definition.name, failureReason(reader));
		if (header.opcode === Opcode.Event) {
			const [decoded, value] = pcall(decodeBody, definition.input, "payload", reader);
			if (!decoded) {
				if (!missingInstance(value)) this.reject(peer, definition, definition.name, failureReason(value));
				return;
			}
			const receiver = receiverOf(definition);
			if (receiver === undefined) return this.unreceived(definition);
			if (peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(peer, FailReason.Overloaded);
			return this.start(peer, receiver, [player, value]);
		}
		const nonce = header.nonce;
		if (nonce <= peer.lastRequestNonce)
			return this.reject(peer, definition, `${definition.name} with a nonce already used`);
		peer.lastRequestNonce = nonce;
		const refuse = (reason: ErrorReason) =>
			this.fire(peer, controlPacket(Opcode.Error, [header.index, nonce], reason));
		const [decoded, value] = pcall(decodeBody, definition.input, "request", reader);
		if (!decoded) {
			if (!missingInstance(value)) this.reject(peer, definition, definition.name, failureReason(value));
			return refuse(ErrorReason.Invalid);
		}
		const receiver = receiverOf(definition);
		if (receiver === undefined) return refuse(ErrorReason.Unavailable);
		if (peer.counted >= L.MAX_ACTIVE_HANDLERS) return refuse(ErrorReason.Overloaded);
		this.start(peer, receiver, [player, value], { definition, nonce });
	}

	private ready(peer: Peer) {
		peer.state = PeerState.Ready;
		this.flush(peer);
		const player = peer.player!;
		for (const claim of readyClaims()) {
			if (peer.state !== PeerState.Ready) return;
			if (!isLive(claim)) continue;
			const [ok, value] = this.runReady(peer, claim, claim.callback, player);
			if (!ok) continue;
			if (typeIs(value, "function")) this.keep(peer, { claim, cleanup: value });
			else if (value !== undefined)
				this.reportCallback(
					peer,
					claim,
					undefined,
					`returned a ${typeOf(value)}; ClientReady callbacks return nothing or a cleanup function`,
				);
		}
	}
}

function toClient(token: unknown, call: string): Definition {
	const definition = definitionOf(token);
	if (definition === undefined || definition.kind !== "to-client")
		error(`${call} needs a message declared with toClient`, 0);
	return definition;
}
