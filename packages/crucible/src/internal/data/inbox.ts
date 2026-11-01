import { bootClock } from "../boot";
import { append, clearQuarantine, type AppendDecision, type ClearDecision } from "./inbox-transforms";
import { jsonBytes, parseEnvelope, type Envelope } from "./envelope";
import {
	CALL_DEADLINE_SECONDS,
	INBOX_MAX_ATTEMPTS,
	INBOX_RETRY_SECONDS,
	MESSAGE_ID_MAX_AGE_SECONDS,
	MESSAGE_PAYLOAD_MAX_BYTES,
	OPERATION_ID_MAX_BYTES,
} from "./limits";
import { ageOf, createdAt, newOperationId } from "./operation-id";
import type { DataEngine, ProfileRuntime, StoreResult } from "./profiles";
import { Priority } from "./scheduler";
import { Phase, type Session } from "./session";

/** @internal A message payload admitted against a descriptor (rule 76), or refused. */
export type Admitted = { readonly ok: true; readonly value: unknown } | { readonly ok: false };

/** @internal A declared message: its name and the admission of its payload (rule 65). */
export interface MessageDecl {
	readonly name: string;
	readonly admit: (payload: unknown) => Admitted;
}

/** @internal A quarantined item as `quarantined` lists it (rule 73). */
export interface QuarantinedEntry {
	readonly id: string;
	readonly name: string;
	readonly reason: "payload" | "attempts";
	readonly attempts: number;
	readonly sentAt: number;
	readonly payload: unknown;
}

interface Receiver {
	readonly message: MessageDecl;
	readonly receive: (player: Player, data: object, payload: unknown) => object;
}

function retryAfter(attempts: number): number {
	return INBOX_RETRY_SECONDS[math.min(attempts, INBOX_RETRY_SECONDS.size()) - 1];
}

/** @internal One profile store's offline inbox: its receivers, its sends and the session-side delivery (rules 65 to 73). */
export class Inbox {
	private readonly receivers = new Map<string, Receiver>();

	private readonly engine: DataEngine;
	private readonly store: string;

	constructor(private readonly runtime: ProfileRuntime) {
		this.engine = runtime.engine;
		this.store = runtime.schema.name;
	}

	/** Registers the receiver for one message name; a name has at most one receiver on this server (rule 72). */
	receive(message: MessageDecl, receive: (player: Player, data: object, payload: unknown) => object) {
		this.receivers.set(message.name, { message, receive });
	}

	/** Rules 66 to 70. Yields until the id is durably queued or seen, or the call fails. */
	send(userId: number, message: MessageDecl, payload: unknown, id?: string): StoreResult<undefined> {
		const admitted = message.admit(payload);
		if (!admitted.ok) error(`${this.store}: the payload of message ${message.name} is invalid`, 2);
		const bytes = jsonBytes(admitted.value);
		if (bytes > MESSAGE_PAYLOAD_MAX_BYTES)
			error(
				`${this.store}: the payload of message ${message.name} is ${bytes} bytes, over ${MESSAGE_PAYLOAD_MAX_BYTES}`,
				2,
			);
		const port = this.engine.port;
		const sent = id ?? newOperationId(port);
		if (!typeIs(sent, "string") || sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)
			error(
				`${this.store}: a message id comes from operationId() and is at most ${OPERATION_ID_MAX_BYTES} bytes`,
				2,
			);
		if (ageOf(sent, port.wall(), MESSAGE_ID_MAX_AGE_SECONDS) !== "fresh") return { kind: "expired" };
		const item = {
			id: sent,
			name: message.name,
			version: this.runtime.schema.version,
			payload: admitted.value,
			generation: 0,
			attempts: 0,
		};
		let decision: AppendDecision | undefined;
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.store,
			key: tostring(userId),
			priority: Priority.Ordinary,
			kind: "update",
			dispatchDeadline: deadline,
			bytes: bytes + 1024,
			transform: () => (current) => append(current, userId, item, port.wall, (d) => (decision = d)),
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error" || decision === undefined) return { kind: "unavailable" };
		if (decision.kind === "unreadable") return { kind: "unavailable" };
		if (decision.kind !== "appended" && decision.kind !== "duplicate") return { kind: decision.kind };
		this.prompt(userId);
		return { kind: "ok", value: undefined };
	}

	/** A session of this user on this server reads the change within one round trip (rules 66, 73). */
	private prompt(userId: number) {
		for (const [, session] of this.runtime.sessions)
			if (session.userId === userId && session.phase === Phase.Active)
				this.runtime.requestWrite(session, Priority.Renew);
	}

	/** Rule 73: one read at Ordinary priority, with no lock. */
	quarantined(userId: number): StoreResult<readonly QuarantinedEntry[]> {
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.store,
			key: tostring(userId),
			priority: Priority.Ordinary,
			kind: "read",
			dispatchDeadline: deadline,
			bytes: 1024,
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error") return { kind: "unavailable" };
		const parsed = parseEnvelope(settled.outcome.value?.value, "profiles");
		if (parsed.kind === "absent") return { kind: "ok", value: [] };
		if (parsed.kind === "unreadable") return { kind: "unavailable" };
		return {
			kind: "ok",
			value: parsed.envelope.quarantine.map((item) => ({
				id: item.id,
				name: item.name,
				reason: item.reason,
				attempts: item.attempts,
				sentAt: createdAt(item.id) ?? 0,
				payload: item.payload,
			})),
		};
	}

	/** Rule 73: moves listed quarantined items back to the inbox, or into `seen`. Returns how many changed. */
	clearQuarantined(userId: number, ids: readonly string[], action: "retry" | "discard"): StoreResult<number> {
		for (const id of ids)
			if (!typeIs(id, "string") || id.size() > OPERATION_ID_MAX_BYTES)
				error(`${this.store}: a message id is at most ${OPERATION_ID_MAX_BYTES} bytes`, 2);
		let decision: ClearDecision | undefined;
		const listed = new Set(ids);
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.store,
			key: tostring(userId),
			priority: Priority.Ordinary,
			kind: "update",
			dispatchDeadline: deadline,
			bytes: 1024,
			transform: () => (current) => clearQuarantine(current, userId, listed, action, (d) => (decision = d)),
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error" || decision === undefined || decision.kind === "unreadable")
			return { kind: "unavailable" };
		if (decision.kind === "full") return { kind: "full" };
		if (decision.changed > 0) this.prompt(userId);
		return { kind: "ok", value: decision.changed };
	}

	/**
	 * Rules 71 and 72, on every envelope the session reads. Retires what the envelope proves durable, then applies the
	 * items the session has not handled, in stored order.
	 */
	deliver(session: Session, envelope: Envelope) {
		for (const [id, revision] of session.acks) if (revision <= session.confirmed) session.acks.delete(id);
		for (const [id, mark] of session.marks) {
			const item = envelope.inbox.find((other) => other.id === id);
			if (item?.generation === mark.generation) continue;
			if (
				mark.quarantine !== undefined &&
				item?.generation === mark.generation + 1 &&
				item.attempts === mark.attempts
			)
				this.engine.diagnose(
					`quarantine-full:${this.store}:${session.userId}`,
					`quarantine full for ${this.store} ${session.userId}; clear it with clearQuarantined`,
				);
			session.marks.delete(id);
		}
		session.inbox = envelope.inbox;
		this.apply(session);
	}

	private apply(session: Session) {
		if (!this.runtime.accept(session)) return;
		const present = new Set<string>();
		for (const item of session.inbox) present.add(item.id);
		for (const [id] of session.retryAt) if (!present.has(id)) session.retryAt.delete(id);
		let soonest: number | undefined;
		let staged = false;
		for (const item of session.inbox) {
			const now = bootClock();
			const receiver = this.receivers.get(item.name);
			if (receiver === undefined || session.acks.has(item.id) || session.marks.has(item.id)) continue;
			if (item.version > this.runtime.schema.version) {
				this.engine.diagnose(
					`inbox-version:${session.token}:${item.id}`,
					`${this.store}: ${session.userId}'s inbox item ${item.id} was sent by a newer build (version ${item.version}) and stays queued`,
				);
				continue;
			}
			let at = session.retryAt.get(item.id);
			if (at === undefined && item.attempts > 0) {
				at = now + retryAfter(item.attempts);
				session.retryAt.set(item.id, at);
			}
			if (at !== undefined && now < at) {
				soonest = math.min(soonest ?? at, at);
				continue;
			}
			const admitted = receiver.message.admit(item.payload);
			if (!admitted.ok) {
				session.marks.set(item.id, {
					id: item.id,
					generation: item.generation,
					attempts: item.attempts,
					quarantine: "payload",
				});
				session.retryAt.set(item.id, now + retryAfter(INBOX_RETRY_SECONDS.size()));
				this.engine.diagnose(
					`inbox-payload:${session.token}:${item.id}`,
					`${this.store}: ${session.userId}'s inbox item ${item.id} (${item.name}) has a payload this build refuses; quarantined`,
				);
				staged = true;
				continue;
			}
			const [ok] = pcall(() =>
				session.write(
					(data) => receiver.receive(session.player, data, admitted.value),
					() => session.acks.set(item.id, session.revision + 1),
				),
			);
			if (!ok) {
				const attempts = item.attempts + 1;
				session.marks.set(item.id, {
					id: item.id,
					generation: item.generation,
					attempts,
					quarantine: attempts >= INBOX_MAX_ATTEMPTS ? "attempts" : undefined,
				});
				session.retryAt.set(item.id, now + retryAfter(attempts));
				this.engine.diagnose(
					`inbox-attempt:${session.token}:${item.id}`,
					`${this.store}: ${session.userId}'s inbox item ${item.id} (${item.name}) failed attempt ${attempts}`,
				);
				staged = true;
			} else staged = true;
		}
		if (staged) this.runtime.requestWrite(session, Priority.Renew);
		if (soonest !== undefined) {
			const at = soonest;
			this.engine.port.delay(math.max(at - bootClock(), 0), () => this.apply(session));
		}
	}
}
