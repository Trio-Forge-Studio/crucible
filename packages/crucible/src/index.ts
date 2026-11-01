export { Subsystem } from "./subsystem";
export type { CrucibleObject } from "./crucible-object";
export type { All } from "./all";
export { inject } from "./inject";
export { attribute } from "./attribute";
export { CrucibleBehaviour, type BehaviourClass } from "./crucible-behaviour";

export interface BootOptions {
	readonly load: readonly string[];
	readonly fixedRate?: number;
}

export { Logging, LogLevel } from "./logging";
export type { Logger, LogRecord, LogSink, SinkSubscription, LogContext, LogProperties } from "./logging";

export { state, computed, effect, watch, untracked, scope, onCleanup } from "./reactive";
export type { Readable, State } from "./reactive";

export { toServer, toClient, askServer, receive, ClientReady } from "./network";
export type {
	ToServer,
	ToClient,
	AskServer,
	Receiver,
	ReceiveTarget,
	ReceiveCallback,
	AskResult,
	AskFailure,
	AskOptions,
	PayloadArgs,
	f32,
	f64,
	u8,
	u16,
	u32,
} from "./network";
