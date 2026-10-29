/** @internal Ends one engine subscription. Idempotent. */
export type Disconnect = () => void;

/** @internal The RunService events the scheduler connects. `BindToRenderStep` lanes have their own entry. */
export const enum FrameEvent {
	PreRender,
	PreSimulation,
	PostSimulation,
}

/** @internal */
export const enum LogLevel {
	Warning,
	Error,
}

/**
 * @internal
 * Everything the runtime asks of the engine. Coroutine functions are not here: they are the Luau language,
 * identical in Roblox and Lune. The runner cancels a yielded worker with `task.cancel` itself, because the pool
 * works before any port is installed.
 */
export interface EnginePort {
	readonly hasTag: (instance: Instance, tag: string) => boolean;
	readonly tagged: (tag: string) => readonly Instance[];
	readonly onTagAdded: (tag: string, listener: (instance: Instance) => void) => Disconnect;
	readonly onTagRemoved: (tag: string, listener: (instance: Instance) => void) => Disconnect;

	/** `IsDescendantOf(game)`. False once destroyed or streamed out. */
	readonly inWorld: (instance: Instance) => boolean;
	readonly isA: (instance: Instance, className: string) => boolean;
	readonly onAncestryChanged: (instance: Instance, listener: () => void) => Disconnect;

	readonly getAttribute: (instance: Instance, name: string) => unknown;
	readonly onAttributeChanged: (instance: Instance, name: string, listener: () => void) => Disconnect;

	readonly onFrame: (event: FrameEvent, listener: (dt: number) => void) => Disconnect;
	readonly bindRenderStep: (name: string, priority: number, listener: (dt: number) => void) => Disconnect;

	/** `task.defer`. */
	readonly defer: (callback: () => void) => void;
	/** `task.cancel`. The runtime only passes a suspended thread. */
	readonly cancel: (thread: thread) => void;

	readonly log: (level: LogLevel, message: string) => void;
	/** `GetFullName`, for messages. */
	readonly describe: (instance: Instance) => string;
}
