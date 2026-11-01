import type { Subsystem } from "../../subsystem";
import { bootClock, shutdownProgress } from "../boot";
import {
	DISPATCH_CUTOFF_SECONDS,
	FINAL_PHASE_SECONDS,
	SETTLE_MARGIN_SECONDS,
	SHUTDOWN_FALLBACK_SECONDS,
} from "./limits";
import type { DataEngine } from "./profiles";
import { Phase } from "./session";

function pending(engine: DataEngine): boolean {
	for (const runtime of engine.profiles)
		for (const [, session] of runtime.sessions) if (session.phase !== Phase.Ended) return true;
	return false;
}

/**
 * @internal DataRuntime's onShutdown (rules 50 to 56), against core's deadline through E-D1. `subsystem` is the hook's
 * own Subsystem, excluded from `pending`; with no visible sweep the window is SHUTDOWN_FALLBACK_SECONDS.
 */
export function shutdownData(engine: DataEngine, subsystem?: Subsystem) {
	const port = engine.port;
	const progress = subsystem === undefined ? undefined : shutdownProgress(subsystem);
	const deadline = progress?.deadline ?? bootClock() + SHUTDOWN_FALLBACK_SECONDS;
	const finalAt = deadline - FINAL_PHASE_SECONDS;
	const cutoff = deadline - DISPATCH_CUTOFF_SECONDS;
	const settleAt = deadline - SETTLE_MARGIN_SECONDS;
	engine.scheduler.setDispatchCutoff(cutoff);
	engine.closing = true;
	engine.receipts.close();
	for (const runtime of engine.profiles)
		for (const [, session] of runtime.sessions) {
			if (session.phase === Phase.Loading) session.abandoned = true;
			else if (session.phase === Phase.Active && session.revision > session.confirmed) runtime.flush(session);
		}
	engine.wake();
	const others = () => (subsystem === undefined ? 0 : (shutdownProgress(subsystem)?.pending ?? 0));
	while (bootClock() < finalAt && others() > 0) port.wait(0);
	engine.finalPhase = true;
	for (const runtime of engine.profiles)
		for (const [, session] of runtime.sessions)
			if (session.phase === Phase.Active) runtime.release(session, "shutdown");
	while (bootClock() < settleAt && pending(engine)) port.wait(0);
	engine.settled = true;
	engine.wake();
}
