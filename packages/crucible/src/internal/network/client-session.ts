import type { AskFailure, AskResult } from "../../network";
import { runtimeDiagnostic } from "../diagnostics";
import {
	adopt,
	catalogSize,
	type Definition,
	definitionAt,
	definitionOf,
	discardIndices,
	unpublished,
} from "./catalog";
import { closeClaims, freezeClaims, openClaims, receiverOf } from "./claims";
import {
	argumentCount,
	controlPacket,
	decodeBody,
	encodeBody,
	failureReason,
	FailReason,
	frame,
	type Header,
	missingInstance,
	Opcode,
	openBody,
	type Packet,
	readHeader,
} from "./envelope";
import * as L from "./limits";
import type { Cancel } from "./port";
import { newPeer, type Peer, PeerState, Session, Stage } from "./session";

const FAIL_NAMES = ["overflow", "overloaded", "closing", "malformed"];
const ERROR_REASONS: readonly AskFailure[] = ["unavailable", "handler-error", "overloaded", "invalid"];

function failed(reason: AskFailure): AskResult<unknown> {
	return table.freeze({ ok: false, reason });
}

interface Pending {
	readonly definition: Definition;
	readonly thread: thread;
	readonly packet: Packet;
	deadline?: Cancel;
	result?: AskResult<unknown>;
}

type Game = Extract<Header, { readonly payloadAt: number }>;

/** @internal The client's session. Imported only by `src/client/index.ts`. Its one peer is the server. */
export class ClientSession extends Session {
	private readonly peer = newPeer();
	private nextNonce = 1;
	private readonly pending = new Map<number, Pending>();
	/** Callers settled on the receive path, resumed only after its pcall returns (rule 44). */
	private later?: Pending[];
	private fired = false;
	private readonly unpublishedReported = new Set<Definition>();
	private readonly undeclared = new Set<number>();
	private readonly unmatched = new Set<Definition>();
	private readonly undecoded = new Set<Definition>();
	private readonly missingPlayer = new Set<Definition>();

	/** Rule 13: the only protocol check. Any failure throws, so client boot fails before a module loads. */
	open() {
		this.stage = Stage.Open;
		openClaims({ side: "client", retired: () => {} });
		const where = `ReplicatedStorage.${L.REMOTE_NAME}`;
		const found = this.port.discover(L.CLIENT_DISCOVERY_TIMEOUT_SECONDS, (...args) =>
			this.receive(argumentCount(args), args[0], args[1]),
		);
		if (found === undefined)
			error(`Crucible found no ${where} within ${L.CLIENT_DISCOVERY_TIMEOUT_SECONDS} seconds`, 0);
		if (found.className !== "RemoteEvent") error(`Crucible found ${where} as a ${found.className}`, 0);
		if (found.protocol !== L.PROTOCOL_VERSION)
			error(
				`Crucible ${where} has Protocol ${tostring(found.protocol)}; this client needs ${L.PROTOCOL_VERSION}`,
				0,
			);
		const [adopted, err] = pcall(adopt, found.catalog as string);
		if (!adopted) error(`Crucible ${where} has an invalid Catalog: ${tostring(err)}`, 0);
	}

	loaded() {
		this.stage = Stage.Loaded;
	}

	/** Rules 14, 46 and 79: freeze claims, report unpublished receivers, then READY, ready, and the flush. */
	commit() {
		freezeClaims();
		for (const definition of unpublished()) if (receiverOf(definition) !== undefined) this.unpublished(definition);
		if (this.peer.state === PeerState.Waiting) {
			this.fire(this.peer, controlPacket(Opcode.Ready, []));
			this.fired = true;
			this.peer.state = PeerState.Ready;
			this.flush(this.peer);
		}
		this.stage = Stage.Committed;
	}

	close() {
		if (this.stage === Stage.Closed) return;
		const committed = this.stage === Stage.Committed;
		this.stage = Stage.Closed;
		if (this.fired && this.peer.state !== PeerState.Ended)
			pcall(() => this.fire(this.peer, controlPacket(Opcode.Fail, [], FailReason.Closing)));
		pcall(() => this.end(this.peer));
		pcall(() => this.port.disconnect());
		closeClaims(!committed);
		if (!committed) discardIndices();
	}

	send(token: unknown, value: unknown): boolean {
		const sendable = this.sendable();
		const definition = toServer(token, "to-server", "Network.send needs a message declared with toServer");
		const body = encodeBody(definition.name, definition.input, "payload", value);
		if (!sendable || this.peer.state === PeerState.Ended) return false;
		const index = definition.index;
		if (index === undefined) {
			this.unpublished(definition);
			return false;
		}
		return this.deliver(this.peer, frame(body, Opcode.Event, [index]));
	}

	/** Rules 37 to 44. Yields until the reply, the declared deadline or the session ending. */
	ask(token: unknown, value: unknown): AskResult<unknown> {
		const sendable = this.sendable();
		const definition = toServer(token, "ask-server", "Network.ask needs a message declared with askServer");
		const body = encodeBody(definition.name, definition.input, "request", value);
		if (!sendable || this.peer.state === PeerState.Ended) return failed("disconnected");
		const index = definition.index;
		if (index === undefined) {
			this.unpublished(definition);
			return failed("unavailable");
		}
		if (this.pending.size() >= L.MAX_PENDING_ASKS) return failed("overloaded");
		const nonce = this.nextNonce;
		const packet = frame(body, Opcode.Request, [index, nonce]);
		this.nextNonce = nonce + 1;
		const entry: Pending = { definition, thread: coroutine.running(), packet };
		this.pending.set(nonce, entry);
		entry.deadline = this.port.delay(definition.timeout, () => this.expire(nonce));
		this.deliver(this.peer, packet);
		if (entry.result !== undefined) return entry.result;
		return coroutine.yield()[0] as AskResult<unknown>;
	}

	protected fire(_peer: Peer, packet: Packet) {
		this.port.fireServer(packet.bytes, packet.sidecar);
	}

	protected end(peer: Peer): boolean {
		if (!super.end(peer)) return false;
		const nonces = new Array<number>();
		for (const [nonce] of this.pending) nonces.push(nonce);
		nonces.sort();
		for (const nonce of nonces) this.settle(nonce, failed("disconnected"));
		return true;
	}

	/** Rule 77: report once, FAIL best effort, end. The server kicks on that FAIL. */
	protected fail(peer: Peer, reason: FailReason, detail?: string) {
		if (peer.state === PeerState.Ended) return;
		runtimeDiagnostic(
			"error",
			`Crucible network session failed (${FAIL_NAMES[reason]})${detail === undefined ? "" : `: ${detail}`}`,
		);
		pcall(() => this.fire(peer, controlPacket(Opcode.Fail, [], reason)));
		this.end(peer);
	}

	private unpublished(definition: Definition) {
		if (this.unpublishedReported.has(definition)) return;
		this.unpublishedReported.add(definition);
		runtimeDiagnostic("warning", `Crucible ${definition.name} is not in the server's network catalog`);
	}

	private once(seen: Set<Definition>, definition: Definition, message: string) {
		if (seen.has(definition)) return;
		seen.add(definition);
		runtimeDiagnostic("warning", message);
	}

	/** Rule 40: a queued ask that times out leaves the queue without renumbering. */
	private expire(nonce: number) {
		const entry = this.pending.get(nonce);
		if (entry === undefined) return;
		const index = this.peer.queue.indexOf(entry.packet);
		if (index !== -1) {
			this.peer.queue.remove(index);
			this.peer.queuedBytes -= entry.packet.charge;
		}
		this.settle(nonce, failed("timeout"));
	}

	/** Rule 44: the one non-yielding settle. Exactly one of reply, deadline and session end wins. */
	private settle(nonce: number, result: AskResult<unknown>) {
		const entry = this.pending.get(nonce);
		if (entry === undefined) return;
		this.pending.delete(nonce);
		entry.deadline?.();
		entry.result = result;
		if (this.later !== undefined) this.later.push(entry);
		else this.resume(entry);
	}

	private resume(entry: Pending) {
		if (coroutine.status(entry.thread) === "suspended") this.port.resume(entry.thread, entry.result);
	}

	/** OnClientEvent. A protocol violation fails the session (rule 77). */
	private receive(count: number, packet?: unknown, sidecar?: unknown) {
		const later = new Array<Pending>();
		this.later = later;
		const [ok, err] = pcall(() => this.dispatch(count, packet, sidecar));
		this.later = undefined;
		if (!ok) this.fail(this.peer, FailReason.Malformed, `receive failed: ${tostring(err)}`);
		for (const entry of later) this.resume(entry);
	}

	private violation(detail: string) {
		this.fail(this.peer, FailReason.Malformed, detail);
	}

	private dispatch(count: number, packet: unknown, sidecar: unknown) {
		if (this.peer.state === PeerState.Ended) return;
		const header = readHeader(count, packet, sidecar);
		if (header === undefined) return this.violation("a malformed packet");
		if (header.opcode === Opcode.Fail) {
			const reason = FAIL_NAMES[header.reason] ?? `unknown reason ${header.reason}`;
			runtimeDiagnostic("error", `Crucible network session ended: the server sent FAIL (${reason})`);
			this.end(this.peer);
			return;
		}
		if (!this.fired) return this.violation("a game packet before READY");
		if (header.opcode === Opcode.Event)
			return this.event(header, packet as buffer, sidecar as Record<string, Instance>);
		if (header.opcode === Opcode.Ok || header.opcode === Opcode.Error)
			return this.answer(header, packet as buffer, sidecar as Record<string, Instance>);
		this.violation("a client-to-server packet");
	}

	private event(header: Game, packet: buffer, sidecar: Record<string, Instance>) {
		if (header.index >= catalogSize()) return this.violation("an unknown message index");
		const definition = definitionAt(header.index);
		if (definition === undefined) {
			if (this.undeclared.has(header.index)) return;
			this.undeclared.add(header.index);
			runtimeDiagnostic(
				"warning",
				`Crucible dropped catalog message ${header.index}: this client never declared it`,
			);
			return;
		}
		if (definition.kind !== "to-client") return this.violation(`${definition.name} sent as an event`);
		const [decoded, value] = pcall(() =>
			decodeBody(definition.input, "payload", openBody(packet, sidecar, header)),
		);
		if (!decoded) {
			if (missingInstance(value))
				return this.once(this.missingPlayer, definition, `Crucible dropped ${definition.name}: a Player left`);
			return this.violation(`${definition.name} ${failureReason(value)}`);
		}
		const receiver = receiverOf(definition);
		if (receiver === undefined) return this.unreceived(definition);
		if (this.peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(this.peer, FailReason.Overloaded);
		this.start(this.peer, receiver, [value]);
	}

	/** Rule 43: a reply settles the pending call with its nonce, and must name that call's message. */
	private answer(
		header: Extract<Header, { readonly opcode: Opcode.Ok | Opcode.Error }>,
		packet: buffer,
		sidecar: Record<string, Instance>,
	) {
		const definition = definitionAt(header.index);
		if (definition === undefined || definition.kind !== "ask-server")
			return this.violation("a reply naming a message that is not an ask");
		const entry = this.pending.get(header.nonce);
		if (entry === undefined)
			return this.once(
				this.unmatched,
				definition,
				`Crucible ignored a late or unknown reply to ${definition.name}`,
			);
		if (entry.definition !== definition)
			return this.violation(`a reply to ${entry.definition.name} naming ${definition.name}`);
		if (header.opcode === Opcode.Error) {
			const reason = ERROR_REASONS[header.reason];
			if (reason === undefined) return this.violation(`an unknown ERROR reason ${header.reason}`);
			return this.settle(header.nonce, failed(reason));
		}
		const [decoded, value] = pcall(() =>
			decodeBody(definition.output!, "response", openBody(packet, sidecar, header)),
		);
		if (!decoded) {
			this.once(
				this.undecoded,
				definition,
				`Crucible could not decode a reply to ${definition.name}: ${failureReason(value)}`,
			);
			return this.settle(header.nonce, failed("invalid"));
		}
		this.settle(header.nonce, table.freeze({ ok: true, value }));
	}
}

function toServer(token: unknown, kind: "to-server" | "ask-server", message: string): Definition {
	const definition = definitionOf(token);
	if (definition === undefined || definition.kind !== kind) error(message, 0);
	return definition;
}
