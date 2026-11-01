import { bootClock } from "../boot";
import { jsonBytes } from "./envelope";
import {
	LOAD_DEADLINE_SECONDS,
	MAX_PURCHASE_IDS,
	PURCHASE_ID_MAX_BYTES,
	RECEIPT_SAVE_TIMEOUT_SECONDS,
	RECEIPTS_MAX_BYTES,
} from "./limits";
import type { DataEngine, ProfileRuntime } from "./profiles";
import { Priority } from "./scheduler";
import { Phase, type Session } from "./session";
import { Waiters } from "./waiters";

/** @internal The part of Roblox's receipt info that data reads. */
export interface ReceiptInfo {
	readonly PlayerId: number;
	readonly PurchaseId: string;
	readonly ProductId: number;
	readonly CurrencySpent: number;
}

/** @internal A developer product purchase, as a grant callback sees it. */
export interface Purchase {
	readonly id: string;
	readonly productId: number;
	readonly currencySpent: number;
}

/** @internal Stages an effect that runs once, after the grant is durable (rule 61). */
export type AfterGrant = (effect: () => void) => void;

/** @internal */
export type ReceiptResult = "granted" | "notYet";

/** @internal The profile store and grant callback that handle one developer product. */
export interface GrantRoute {
	readonly runtime: ProfileRuntime;
	readonly grant: (player: Player, data: object, purchase: Purchase, afterGrant: AfterGrant) => object;
}

/** @internal MarketplaceService, as far as the router uses it. */
export interface Marketplace {
	ProcessReceipt: (info: ReceiptInfo) => Enum.ProductPurchaseDecision;
}

class Pending {
	result?: ReceiptResult;
	readonly waiters: Waiters;

	constructor(engine: DataEngine) {
		this.waiters = new Waiters(engine.port);
	}
}

/**
 * @internal Crucible's ProcessReceipt (rules 57 to 64). Idempotency rests on two facts: a purchase id and its grant's
 * data change are one revision, and "granted" is returned only once an owned write with the session's token is
 * confirmed to hold the id. Purchase ids are never evicted.
 */
export class ReceiptRouter {
	private readonly routes = new Map<number, GrantRoute>();
	private readonly pending = new Map<string, Pending>();
	private closed = false;
	private service?: Marketplace;

	constructor(private readonly engine: DataEngine) {}

	/** Rule 104: DataRuntime registers one route per developer product at activation. */
	route(productId: number, route: GrantRoute) {
		this.routes.set(productId, route);
	}

	/** A product whose grant claim is gone is refused like an unknown one (rule 58). */
	unroute(productId: number) {
		this.routes.delete(productId);
	}

	/** Rule 57: sets ProcessReceipt to this router. */
	install(service: Marketplace) {
		this.service = service;
		service.ProcessReceipt = (info) => this.decision(this.process(info));
	}

	private decision(result: ReceiptResult): Enum.ProductPurchaseDecision {
		return result === "granted"
			? Enum.ProductPurchaseDecision.PurchaseGranted
			: Enum.ProductPurchaseDecision.NotProcessedYet;
	}

	/**
	 * Shutdown hook start and retirement: new callbacks get NotProcessedYet at once and held ones wake and return it
	 * (rules 51, 63). Retirement also replaces the installed handler (rules 57, 107).
	 */
	close(retire = false) {
		this.closed = true;
		if (retire && this.service !== undefined)
			this.service.ProcessReceipt = () => Enum.ProductPurchaseDecision.NotProcessedYet;
		this.engine.wake();
	}

	/** Rules 58 to 64. Yields while a purchase is held for its load or waits for its save. */
	process(info: ReceiptInfo): ReceiptResult {
		const engine = this.engine;
		const holdUntil = bootClock() + LOAD_DEADLINE_SECONDS;
		while (true) {
			if (this.closed) return "notYet";
			const route = this.routes.get(info.ProductId);
			if (route === undefined) {
				engine.diagnose(
					`product:${info.ProductId}`,
					`a purchase of product ${info.ProductId} was refused: no grant handles it`,
				);
				return "notYet";
			}
			const id = info.PurchaseId;
			if (!typeIs(id, "string") || id.size() < 1 || id.size() > PURCHASE_ID_MAX_BYTES) {
				engine.diagnose(
					`receipt-id:${info.ProductId}`,
					`a purchase of product ${info.ProductId} was refused: its purchase id is not 1 to ${PURCHASE_ID_MAX_BYTES} bytes`,
				);
				return "notYet";
			}
			const player = engine.port.players.byUserId(info.PlayerId);
			if (player === undefined) return "notYet";
			const runtime = route.runtime;
			const session = runtime.sessions.get(player);
			if (session === undefined || session.phase === Phase.Loading || session.sealedBy === "handoff") {
				if (bootClock() >= holdUntil) return "notYet";
				runtime.changed.park(holdUntil);
				continue;
			}
			if (!runtime.writable(player)) return "notYet";
			const key = `${info.PlayerId}:${id}`;
			const shared = this.pending.get(key);
			if (shared !== undefined) {
				while (shared.result === undefined) shared.waiters.park();
				return shared.result;
			}
			if (session.durable.has(id)) return this.granted(session, id);
			const pending = new Pending(engine);
			this.pending.set(key, pending);
			const [ok, result] = pcall(() => this.lead(route, session, player, id, info));
			pending.result = ok ? result : "notYet";
			this.pending.delete(key);
			pending.waiters.wake();
			if (!ok) error(result, 0);
			return result;
		}
	}

	private lead(route: GrantRoute, session: Session, player: Player, id: string, info: ReceiptInfo): ReceiptResult {
		const runtime = route.runtime;
		const engine = this.engine;
		if (!session.receipts.includes(id)) {
			if (
				session.receipts.size() >= MAX_PURCHASE_IDS ||
				jsonBytes(session.receipts) + jsonBytes(id) + 1 > RECEIPTS_MAX_BYTES
			) {
				engine.diagnose(
					`receipt-capacity:${session.token}`,
					`receipt capacity reached for ${runtime.schema.name} ${session.userId}`,
				);
				return "notYet";
			}
			const purchase = { id, productId: info.ProductId, currencySpent: info.CurrencySpent };
			const effects = new Array<() => void>();
			const afterGrant: AfterGrant = (effect) => {
				if (!typeIs(effect, "function")) error("afterGrant takes a function", 2);
				effects.push(effect);
			};
			const [ok, wrote] = pcall(() =>
				runtime.write(
					player,
					(data) => route.grant(player, data, purchase, afterGrant),
					() => {
						session.stage(id);
						session.effects.set(id, effects);
					},
				),
			);
			if (!ok) {
				engine.diagnose(
					`grant:${runtime.schema.name}:${info.ProductId}`,
					`${runtime.schema.name}: the grant for product ${info.ProductId} threw, yielded or returned an invalid value; nothing was stored`,
				);
				return "notYet";
			}
			if (!wrote) return "notYet";
		}
		runtime.requestWrite(session, Priority.Receipt);
		const timeout = bootClock() + RECEIPT_SAVE_TIMEOUT_SECONDS;
		while (!session.durable.has(id) && session.phase !== Phase.Ended && bootClock() < timeout && !engine.settled)
			runtime.changed.park(timeout);
		return session.durable.has(id) ? this.granted(session, id) : "notYet";
	}

	/** Rule 61: the effects staged with this session's grant run once each, on fresh threads. */
	private granted(session: Session, id: string): ReceiptResult {
		const effects = session.effects.get(id);
		session.effects.delete(id);
		for (const effect of effects ?? [])
			this.engine.port.spawn(() => {
				const [ok] = pcall(effect);
				if (!ok)
					this.engine.diagnose(
						`effect:${session.token}:${id}`,
						`an afterGrant effect for ${session.schema.name} ${session.userId} threw`,
					);
			});
		return "granted";
	}
}
