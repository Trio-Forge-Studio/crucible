import { bootClock } from "../boot";
import { runtimeDiagnostic } from "../diagnostics";
import { loadRecord, type Schema } from "./admission";
import { jsonBytes, parseEnvelope, type Envelope } from "./envelope";
import {
	abandonAcquisition,
	acquire,
	ownedWrite,
	type AcquireDecision,
	type OwnedDecision,
	type Sealed,
} from "./lease";
import {
	CALL_DEADLINE_SECONDS,
	HANDOFF_TIMEOUT_SECONDS,
	LEASE_SECONDS,
	LOAD_BUSY_MESSAGE,
	LOAD_DEADLINE_SECONDS,
	LOAD_FAILED_MESSAGE,
	LOAD_POLL_JITTER,
	LOAD_POLL_MIN_SECONDS,
	LOAD_POLL_SCHEDULE_SECONDS,
	LOCAL_AUTHORITY_SECONDS,
	RENEW_INTERVAL_SECONDS,
	RENEW_JITTER_SECONDS,
	SAVE_TIMEOUT_DEFAULT_SECONDS,
	SAVE_TIMEOUT_MAX_SECONDS,
	SERVER_UPDATING_MESSAGE,
	SESSION_ENDED_MESSAGE,
	TAKEOVER_GRACE_SECONDS,
} from "./limits";
import type { DataPort } from "./port";
import { KeyedScheduler, Priority, type Settled } from "./scheduler";
import { assertOutsideCallback, Phase, Session, type Released, type SealReason } from "./session";
import { Waiters } from "./waiters";

/** @internal */
export type LoadReason = "join" | "resume";
/** @internal */
export type ReleaseReason = "leave" | "handoff" | "shutdown";
/** @internal */
export type SaveResult = "ok" | "unavailable" | "timeout" | "uncertain";
/** @internal */
export type StoreResult<T> =
	| { readonly kind: "ok"; readonly value: T }
	| { readonly kind: "timeout" | "uncertain" | "overloaded" | "closing" | "unavailable" };

/**
 * @internal The seams later units fill. D4 runs the claims and owns the slot's node; D2 delivers the inbox and receipts
 * from every envelope the session reads. Each defaults to doing nothing.
 */
export interface ProfileHooks {
	readonly onLoad: (session: Session, value: object, reason: LoadReason) => object;
	readonly onRelease: (session: Session, value: object, reason: ReleaseReason) => object;
	readonly publish: (player: Player, value: object | undefined) => void;
	readonly delivered: (session: Session, envelope: Envelope) => void;
}

const NO_HOOKS: ProfileHooks = {
	onLoad: (_, value) => value,
	onRelease: (_, value) => value,
	publish: () => {},
	delivered: () => {},
};

/** @internal One server's data engine: its port, its scheduler and its shutdown state (rules 32, 50 to 56, 107). */
export class DataEngine {
	readonly scheduler: KeyedScheduler;
	readonly profiles = new Array<ProfileRuntime>();
	/** Loads closed (rule 51). */
	closing = false;
	/** Profile writes refused (rule 53). */
	finalPhase = false;
	/** Waiters settle and renewals stop (rule 54). */
	settled = false;
	private readonly diagnosed = new Set<string>();

	constructor(readonly port: DataPort) {
		this.scheduler = new KeyedScheduler(port, (key, message) => this.diagnose(key, message));
	}

	/** Rule 110: once per key; messages carry store, user, reason, revisions and sizes, never values. */
	diagnose(key: string, message: string) {
		if (this.diagnosed.has(key)) return;
		this.diagnosed.add(key);
		runtimeDiagnostic("warning", message);
	}

	profile(schema: Schema, hooks: ProfileHooks = NO_HOOKS): ProfileRuntime {
		const runtime = new ProfileRuntime(this, schema, hooks);
		this.profiles.push(runtime);
		return runtime;
	}

	wake() {
		for (const runtime of this.profiles) runtime.changed.wake();
	}

	/** Rule 107's session steps: non-yielding; each player is kicked once their release settles. */
	retire() {
		this.settled = true;
		for (const runtime of this.profiles)
			for (const [, session] of runtime.sessions) {
				if (session.phase === Phase.Loading) session.abandoned = true;
				else if (session.phase === Phase.Active) {
					runtime.seal(session, "retired");
					runtime.startRelease(session);
				}
			}
		this.wake();
	}
}

/** @internal The durability engine behind one profile store (rules 9 to 49). */
export class ProfileRuntime {
	readonly sessions = new Map<Player, Session>();
	readonly changed: Waiters;
	/** This server's sealed sessions per user, by token, kept for rule 29 until superseded or expired. */
	private readonly sealed = new Map<number, Map<string, Session>>();

	constructor(
		readonly engine: DataEngine,
		readonly schema: Schema,
		readonly hooks: ProfileHooks,
	) {
		this.changed = new Waiters(engine.port);
	}

	private key(userId: number) {
		return tostring(userId);
	}

	private present(player: Player) {
		return this.engine.port.players.present(player);
	}

	/** Rule 9: writable exactly while active and before the safe bound; a passed bound seals. */
	accept(session: Session): boolean {
		if (session.phase !== Phase.Active) return false;
		if (bootClock() < session.safeUntil) return true;
		this.lose(session, "authority");
		return false;
	}

	writable(player: Player): boolean {
		const session = this.sessions.get(player);
		return session !== undefined && this.accept(session);
	}

	/** Rule 10. */
	players(): readonly Player[] {
		const result = new Array<Player>();
		for (const [player, session] of this.sessions) if (session.writable()) result.push(player);
		return result;
	}

	/** Rule 12: the one write gate. `stage` stages an extra, such as a purchase id, in the same revision. */
	write(player: Player, change: (current: object) => object, stage?: () => void): boolean {
		const session = this.sessions.get(player);
		session?.assertIdle();
		assertOutsideCallback();
		if (this.engine.finalPhase) {
			this.engine.diagnose(
				`late:${this.schema.name}`,
				`${this.schema.name} refused a write after the shutdown final phase; a shutdown hook that writes profiles must finish before the sweep's deadline - 10 s`,
			);
			return false;
		}
		if (session === undefined || !this.accept(session)) return false;
		session.write(change, stage);
		return true;
	}

	private newSession(player: Player): Session {
		const session = new Session(this.schema, player, player.UserId, this.engine.port.guid(), {
			changed: () => this.changed.wake(),
			publish: (value) => this.hooks.publish(player, value),
		});
		this.sessions.set(player, session);
		this.changed.wake();
		return session;
	}

	private supersedes(userId: number): ReadonlyMap<string, Sealed> {
		const result = new Map<string, Sealed>();
		for (const [token, session] of this.sealed.get(userId) ?? new Map<string, Session>())
			if (session.released !== undefined)
				result.set(token, { epoch: session.epoch, snapshot: session.snapshot() });
		return result;
	}

	private stopped(session: Session): boolean {
		return session.abandoned || this.engine.closing || !this.present(session.player);
	}

	private end(session: Session, mayHaveCommitted: boolean, message?: string): boolean {
		session.phase = Phase.Ended;
		if (mayHaveCommitted)
			this.engine.scheduler.submit({
				store: this.schema.name,
				key: this.key(session.userId),
				priority: Priority.Release,
				kind: "update",
				dispatchDeadline: bootClock() + CALL_DEADLINE_SECONDS,
				bytes: 512,
				transform: () => (current) => abandonAcquisition(current, session.userId, session.token, () => {}),
			});
		if (message !== undefined) this.engine.port.players.kick(session.player, message);
		this.drop(session);
		this.changed.wake();
		return false;
	}

	private drop(session: Session) {
		if (this.sessions.get(session.player) === session && !this.present(session.player))
			this.sessions.delete(session.player);
	}

	/** Rules 17 to 21, 29 and 30. Yields; true once the session is writable. */
	load(player: Player, reason: LoadReason): boolean {
		const existing = this.sessions.get(player);
		if (existing !== undefined && (existing.phase === Phase.Loading || existing.phase === Phase.Active))
			return this.await(player);
		const port = this.engine.port;
		const deadline = bootClock() + LOAD_DEADLINE_SECONDS;
		const session = this.newSession(player);
		const userId = session.userId;
		let polls = 0;
		let held = false;
		let mayHaveCommitted = false;
		let earliest: number | undefined;
		let envelope: Envelope | undefined;
		while (envelope === undefined) {
			if (this.stopped(session)) return this.end(session, mayHaveCommitted);
			if (bootClock() >= deadline)
				return this.end(session, mayHaveCommitted, held ? LOAD_BUSY_MESSAGE : LOAD_FAILED_MESSAGE);
			let decision: AcquireDecision | undefined;
			let sentAt = 0;
			const ticket = this.engine.scheduler.submit({
				store: this.schema.name,
				key: this.key(userId),
				priority: Priority.Load,
				kind: "update",
				dispatchDeadline: deadline,
				bytes: 1024,
				// A sealed session's release holds this key's lane until it settles, backoff included, so the
				// supersede set read at dispatch is settled (rules 29, 30, 47).
				transform: () => {
					const input = {
						userId,
						token: session.token,
						serverId: port.serverId,
						jobId: port.jobId,
						schemaVersion: this.schema.version,
						supersedes: this.supersedes(userId),
						wall: port.wall,
					};
					return (current) => acquire(current, input, (d) => (decision = d));
				},
				onDispatch: (at) => (sentAt = at),
				onResult: (outcome) => {
					if (outcome.kind === "error" && !outcome.ambiguous) return;
					earliest ??= sentAt;
					mayHaveCommitted = true;
				},
			});
			ticket.onSettled(() => this.changed.wake());
			while (ticket.settled === undefined && !this.stopped(session)) this.changed.park();
			const settled = ticket.settled;
			if (settled === undefined) return this.end(session, true);
			if (settled.kind === "done" && settled.outcome.kind === "error")
				return this.end(session, mayHaveCommitted, LOAD_FAILED_MESSAGE);
			if (settled.kind !== "done") {
				held = false;
				if (settled.kind === "overloaded") {
					const wake = math.min(deadline, bootClock() + LOAD_POLL_MIN_SECONDS);
					while (bootClock() < wake && !this.stopped(session)) this.changed.park(wake);
				}
				continue;
			}
			const answer = decision!;
			if (answer.kind === "taken" || answer.kind === "already") envelope = answer.envelope;
			else if (answer.kind === "newer") return this.end(session, false, SERVER_UPDATING_MESSAGE);
			else if (answer.kind === "unreadable") {
				this.engine.diagnose(
					`unreadable:${this.schema.name}:${userId}`,
					`${this.schema.name}: ${userId}'s record is unreadable: ${answer.reason}`,
				);
				return this.end(session, false, LOAD_FAILED_MESSAGE);
			} else {
				held = true;
				mayHaveCommitted = false;
				earliest = undefined;
				const base = LOAD_POLL_SCHEDULE_SECONDS[math.min(polls, LOAD_POLL_SCHEDULE_SECONDS.size() - 1)];
				polls += 1;
				const jittered = base * (1 - LOAD_POLL_JITTER + 2 * LOAD_POLL_JITTER * port.random());
				const room = answer.lease.expiresAt + TAKEOVER_GRACE_SECONDS - port.wall();
				const wake = math.min(
					deadline,
					bootClock() + math.max(LOAD_POLL_MIN_SECONDS, math.min(jittered, room)),
				);
				while (bootClock() < wake && !this.stopped(session)) this.changed.park(wake);
			}
		}
		session.epoch = envelope.epoch;
		session.safeUntil = earliest! + LOCAL_AUTHORITY_SECONDS;
		const superseded = this.sealed.get(userId);
		if (superseded !== undefined)
			for (const [token, sealed] of superseded) if (sealed.released !== undefined) superseded.delete(token);
		const loaded = loadRecord(this.schema, envelope.payload, envelope.schemaVersion);
		if (loaded.kind !== "ok") {
			this.engine.diagnose(
				`unreadable:${this.schema.name}:${userId}`,
				`${this.schema.name}: ${userId}'s record did not load: ${loaded.kind}`,
			);
			return this.end(session, true, loaded.kind === "newer" ? SERVER_UPDATING_MESSAGE : LOAD_FAILED_MESSAGE);
		}
		session.value = loaded.value;
		session.revision = loaded.migrated ? 1 : 0;
		for (const id of envelope.receipts) session.stage(id);
		session.durable = new Set(envelope.receipts);
		const [ok, claimed] = pcall(() => {
			const result = this.hooks.onLoad(session, session.value, reason);
			return rawequal(result, session.value) ? result : session.prepare(result);
		});
		if (!ok) {
			this.engine.diagnose(
				`onLoad:${this.schema.name}:${session.token}`,
				`${this.schema.name}: ${userId}'s onLoad failed (${reason})`,
			);
			return this.end(session, true, LOAD_FAILED_MESSAGE);
		}
		if (!rawequal(claimed, session.value)) session.commit(claimed);
		if (this.stopped(session)) return this.end(session, true);
		if (bootClock() >= session.safeUntil) return this.end(session, true, SESSION_ENDED_MESSAGE);
		session.phase = Phase.Active;
		this.hooks.publish(player, session.value);
		this.hooks.delivered(session, envelope);
		session.renewAt = bootClock() + port.random() * RENEW_INTERVAL_SECONDS;
		this.armSeal(session);
		this.armRenewal(session);
		if (session.revision > 0) this.requestWrite(session, Priority.Renew);
		this.changed.wake();
		return true;
	}

	private armSeal(session: Session) {
		const port = this.engine.port;
		const check = () => {
			if (session.phase !== Phase.Active) return;
			const now = bootClock();
			if (now < session.safeUntil) {
				port.delay(session.safeUntil - now, check);
				return;
			}
			while (session.busy) this.changed.park();
			this.lose(session, "authority");
		};
		port.delay(session.safeUntil - bootClock(), check);
	}

	private armRenewal(session: Session) {
		const port = this.engine.port;
		const tick = () => {
			if (session.phase !== Phase.Active || this.engine.settled) return;
			const now = bootClock();
			if (now >= session.renewAt) {
				if (!this.present(session.player)) {
					this.leave(session.player);
					return;
				}
				this.requestWrite(session, Priority.Renew);
			}
			port.delay(math.max(session.renewAt - bootClock(), 1), tick);
		};
		port.delay(math.max(session.renewAt - bootClock(), 0), tick);
	}

	/** Seals at once: the slot reads undefined, writes return false and waiters wake (rule 26). */
	seal(session: Session, reason: SealReason) {
		if (session.phase === Phase.Loading) session.abandoned = true;
		if (session.phase !== Phase.Active) return;
		session.phase = Phase.Sealed;
		session.sealedBy = reason;
		this.hooks.publish(session.player, undefined);
		let tokens = this.sealed.get(session.userId);
		if (tokens === undefined) {
			tokens = new Map();
			this.sealed.set(session.userId, tokens);
		}
		tokens.set(session.token, session);
		if (reason === "authority" || reason === "fenced") {
			this.engine.diagnose(
				`authority:${this.schema.name}:${session.token}`,
				`${this.schema.name}: ${session.userId}'s session lost authority (${reason}) at revision ${session.revision}, confirmed ${session.confirmed}`,
			);
			if (this.present(session.player)) this.engine.port.players.kick(session.player, SESSION_ENDED_MESSAGE);
		}
		this.changed.wake();
	}

	private lose(session: Session, reason: SealReason) {
		this.seal(session, reason);
		this.startRelease(session);
	}

	startRelease(session: Session) {
		session.releaseWanted = true;
		this.pumpWriter(session);
	}

	/** Rule 51's early flush: a Release-priority save that keeps the lease. */
	flush(session: Session) {
		this.requestWrite(session, Priority.Release);
	}

	private requestWrite(session: Session, priority: Priority) {
		session.want = session.want === undefined ? priority : math.min(session.want, priority);
		this.pumpWriter(session);
	}

	/** One owned operation at a time per session; requests coalesce into the next (rules 28, 31, 32). */
	private pumpWriter(session: Session) {
		if (session.writing !== undefined) return;
		const release = session.releaseWanted && session.released === undefined && session.phase === Phase.Sealed;
		if (!release && (session.want === undefined || session.phase !== Phase.Active)) return;
		const priority = release ? Priority.Release : session.want!;
		session.want = undefined;
		const port = this.engine.port;
		const op = ++session.op;
		let decision: OwnedDecision | undefined;
		let sentAt = 0;
		let earliest: number | undefined;
		if (!release)
			session.renewAt = bootClock() + RENEW_INTERVAL_SECONDS + (port.random() * 2 - 1) * RENEW_JITTER_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.schema.name,
			key: this.key(session.userId),
			priority,
			kind: "update",
			renewal: !release,
			safeUntil: session.safeUntil,
			// A release retries until its lease would expire, never only until the safe bound (rule 26, ruling K15).
			dispatchDeadline: release ? session.safeUntil - LOCAL_AUTHORITY_SECONDS + LEASE_SECONDS : session.safeUntil,
			bytes: jsonBytes(session.value) + 1024,
			transform: () => {
				const snapshot = session.snapshot();
				session.carrying = snapshot.revision;
				const input = {
					userId: session.userId,
					token: session.token,
					epoch: session.epoch,
					op,
					snapshot,
					release,
					wall: port.wall,
				};
				return (current) => ownedWrite(current, input, (d) => (decision = d));
			},
			onDispatch: (at) => (sentAt = at),
			onResult: (outcome) => {
				if (outcome.kind === "ok" || outcome.ambiguous) earliest ??= sentAt;
			},
		});
		session.writing = ticket;
		ticket.onSettled((settled) => this.settleOwned(session, release, settled, decision, earliest));
	}

	private settleOwned(
		session: Session,
		release: boolean,
		settled: Settled,
		decision: OwnedDecision | undefined,
		earliest: number | undefined,
	) {
		const ambiguous = session.writing?.ambiguous === true;
		session.writing = undefined;
		let outcome: Released = "given up";
		let lost: SealReason | undefined;
		if (settled.kind === "done" && settled.outcome.kind === "ok" && decision !== undefined) {
			if (decision.kind === "written" || decision.kind === "already") {
				outcome = "committed";
				this.confirm(session, decision.envelope, earliest);
			} else if (decision.kind === "fenced" || decision.kind === "expired") {
				outcome = "fenced";
				lost = decision.kind === "fenced" ? "fenced" : "authority";
			} else {
				this.engine.diagnose(
					`write:${this.schema.name}:${decision.kind}`,
					`${this.schema.name}: ${session.userId}'s owned write was refused: ${decision.kind}`,
				);
				lost = "authority";
			}
		} else if (settled.kind === "uncertain") session.lost = math.max(session.lost, session.carrying);
		if (release) {
			session.released = outcome;
			session.releaseAmbiguous = outcome !== "committed" && ambiguous;
			session.phase = Phase.Ended;
			this.released(session);
		} else if (lost !== undefined) this.lose(session, lost);
		this.changed.wake();
		this.pumpWriter(session);
	}

	private released(session: Session) {
		if (session.released !== "committed")
			this.engine.diagnose(
				`release:${this.schema.name}:${session.token}`,
				`${this.schema.name}: ${session.userId}'s release did not confirm (${session.released}); revisions ${session.confirmed + 1} to ${session.revision} unconfirmed`,
			);
		if (session.sealedBy === "retired" && this.present(session.player))
			this.engine.port.players.kick(session.player, SESSION_ENDED_MESSAGE);
		const tokens = this.sealed.get(session.userId);
		this.engine.port.delay(LEASE_SECONDS + TAKEOVER_GRACE_SECONDS, () => {
			if (tokens?.get(session.token) !== session) return;
			tokens.delete(session.token);
			if (tokens.size() === 0 && this.sealed.get(session.userId) === tokens) this.sealed.delete(session.userId);
		});
		this.drop(session);
	}

	/** Rule 28: confirms exactly what the stored envelope holds, for this token only. */
	private confirm(session: Session, envelope: Envelope, earliest: number | undefined) {
		session.confirmed = math.max(session.confirmed, envelope.commit!.revision);
		session.durable = new Set(envelope.receipts);
		if (earliest !== undefined && this.accept(session))
			session.safeUntil = math.max(session.safeUntil, earliest + LOCAL_AUTHORITY_SECONDS);
		this.hooks.delivered(session, envelope);
	}

	private releaseClaims(session: Session, reason: ReleaseReason) {
		const [ok, claimed] = pcall(() => {
			const result = this.hooks.onRelease(session, session.value, reason);
			return rawequal(result, session.value) ? result : session.prepare(result);
		});
		if (!ok)
			this.engine.diagnose(
				`onRelease:${this.schema.name}:${session.token}`,
				`${this.schema.name}: ${session.userId}'s onRelease (${reason}) was skipped`,
			);
		else if (!rawequal(claimed, session.value)) session.commit(claimed);
	}

	/** Rule 43 on PlayerRemoving, rule 44's self-healing, and rule 53's final phase. */
	release(session: Session, reason: ReleaseReason) {
		while (session.busy) this.changed.park();
		if (session.phase === Phase.Loading) {
			session.abandoned = true;
			this.changed.wake();
			return;
		}
		if (session.phase === Phase.Active) {
			this.releaseClaims(session, reason);
			this.seal(session, reason);
			this.startRelease(session);
		} else if (session.phase === Phase.Ended) this.drop(session);
	}

	leave(player: Player) {
		const session = this.sessions.get(player);
		if (session !== undefined) this.release(session, "leave");
		this.changed.wake();
	}

	/** Rule 45. */
	handoff(player: Player): SaveResult {
		const session = this.sessions.get(player);
		session?.assertIdle();
		assertOutsideCallback();
		if (session === undefined || !this.accept(session)) return "unavailable";
		this.seal(session, "handoff");
		this.releaseClaims(session, "handoff");
		this.startRelease(session);
		const deadline = bootClock() + HANDOFF_TIMEOUT_SECONDS;
		while (session.released === undefined && bootClock() < deadline && !this.engine.settled)
			this.changed.park(deadline);
		if (session.released === "committed") return "ok";
		const pending = session.writing;
		return session.releaseAmbiguous || (pending?.dispatched === true && (pending.inFlight || pending.ambiguous))
			? "uncertain"
			: "timeout";
	}

	/** Rule 47. */
	resume(player: Player): boolean {
		const session = this.sessions.get(player);
		session?.assertIdle();
		assertOutsideCallback();
		if (session !== undefined && this.accept(session)) return true;
		if (!this.present(player)) return false;
		return this.load(player, "resume");
	}

	/** Rule 11. */
	await(player: Player): boolean {
		const deadline = bootClock() + LOAD_DEADLINE_SECONDS;
		while (true) {
			const session = this.sessions.get(player);
			if (session !== undefined && this.accept(session)) return true;
			if (!this.present(player) || this.engine.closing || bootClock() >= deadline) return false;
			if (session !== undefined && session.phase !== Phase.Loading) return false;
			this.changed.park(deadline);
		}
	}

	/** Rule 37: per token, so a new session's first save waits for its own commit. */
	save(player: Player, timeoutSeconds = SAVE_TIMEOUT_DEFAULT_SECONDS): SaveResult {
		assertOutsideCallback();
		const session = this.sessions.get(player);
		if (session === undefined || !this.accept(session)) return "unavailable";
		const deadline = bootClock() + math.clamp(timeoutSeconds, 0, SAVE_TIMEOUT_MAX_SECONDS);
		const target = session.revision;
		this.requestWrite(session, Priority.Renew);
		while (
			session.confirmed < target &&
			session.phase !== Phase.Ended &&
			bootClock() < deadline &&
			!this.engine.settled
		)
			this.changed.park(deadline);
		if (session.confirmed >= target) return "ok";
		return session.uncertain() >= target ? "uncertain" : "timeout";
	}

	/** Rule 49: never opens or writes anything. */
	inspect(userId: number): StoreResult<object | undefined> {
		for (const [, session] of this.sessions)
			if (session.userId === userId && this.accept(session)) return { kind: "ok", value: session.value };
		const deadline = bootClock() + CALL_DEADLINE_SECONDS;
		const ticket = this.engine.scheduler.submit({
			store: this.schema.name,
			key: this.key(userId),
			priority: Priority.Ordinary,
			kind: "read",
			dispatchDeadline: deadline,
			bytes: 1024,
		});
		const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };
		if (settled.kind !== "done") return { kind: settled.kind };
		if (settled.outcome.kind === "error") return { kind: "unavailable" };
		const parsed = parseEnvelope(settled.outcome.value?.value, "profiles");
		if (parsed.kind === "absent" || (parsed.kind === "ok" && parsed.envelope.payload === undefined))
			return { kind: "ok", value: undefined };
		if (parsed.kind === "unreadable") return { kind: "unavailable" };
		const loaded = loadRecord(this.schema, parsed.envelope.payload, parsed.envelope.schemaVersion);
		return loaded.kind === "ok" ? { kind: "ok", value: loaded.value } : { kind: "unavailable" };
	}
}
