import { FrameEvent, LogLevel, type EnginePort } from "./port";

/** @internal */
export interface BootPort extends EnginePort {
	readonly bindToClose: (callback: () => void) => void;
	readonly spawn: (callback: () => void) => void;
	readonly clock: () => number;
	readonly wait: () => void;
}

/** @internal */
export function robloxPort(): BootPort {
	const runService = game.GetService("RunService");
	const events = {
		[FrameEvent.PreRender]: runService.PreRender,
		[FrameEvent.PreSimulation]: runService.PreSimulation,
		[FrameEvent.PostSimulation]: runService.PostSimulation,
		[FrameEvent.RenderStepped]: runService.RenderStepped,
		[FrameEvent.PreAnimation]: runService.PreAnimation,
		[FrameEvent.Heartbeat]: runService.Heartbeat,
	};
	return {
		...robloxBindingPort(),
		onFrame: (event, listener) => {
			const connection = events[event].Connect(listener);
			return () => connection.Disconnect();
		},
		bindRenderStep: (name, priority, listener) => {
			runService.BindToRenderStep(name, priority, listener);
			return () => runService.UnbindFromRenderStep(name);
		},
		defer: (callback) => {
			task.defer(callback);
		},
		cancel: (thread) => task.cancel(thread),
		spawn: (callback) => {
			task.spawn(callback);
		},
		clock: () => os.clock(),
		wait: () => {
			task.wait();
		},
		bindToClose: (callback) => game.BindToClose(callback),
		log: (level, message) => {
			if (level === LogLevel.Error) task.spawn(() => error(`[Crucible] ${message}`, 0));
			else warn(`[Crucible] ${message}`);
		},
	};
}

/** @internal */
export type RobloxBindingPort = Pick<
	EnginePort,
	| "hasTag"
	| "tagged"
	| "onTagAdded"
	| "onTagRemoved"
	| "inWorld"
	| "isA"
	| "bindingScope"
	| "onAncestryChanged"
	| "getAttribute"
	| "onAttributeChanged"
	| "describe"
>;

/** @internal */
export function robloxBindingPort(): RobloxBindingPort {
	const collection = game.GetService("CollectionService");
	const excluded: Instance[] = [
		game.GetService("ServerStorage"),
		game.GetService("ReplicatedStorage"),
		game.GetService("ServerScriptService"),
		game.GetService("StarterGui"),
		game.GetService("StarterPack"),
		game.GetService("StarterPlayer"),
	];
	return {
		hasTag: (instance, tag) => collection.HasTag(instance, tag),
		tagged: (tag) => collection.GetTagged(tag),
		onTagAdded: (tag, listener) => {
			const c = collection.GetInstanceAddedSignal(tag).Connect(listener);
			return () => c.Disconnect();
		},
		onTagRemoved: (tag, listener) => {
			const c = collection.GetInstanceRemovedSignal(tag).Connect(listener);
			return () => c.Disconnect();
		},
		inWorld: (instance) => instance.IsDescendantOf(game),
		isA: (instance, name) => instance.IsA(name as keyof Instances),
		bindingScope: (instance) =>
			!excluded.some((container) => instance === container || instance.IsDescendantOf(container)),
		onAncestryChanged: (instance, listener) => {
			const c = instance.AncestryChanged.Connect(listener);
			return () => c.Disconnect();
		},
		getAttribute: (instance, name) => instance.GetAttribute(name),
		onAttributeChanged: (instance, name, listener) => {
			const c = instance.GetAttributeChangedSignal(name).Connect(listener);
			return () => c.Disconnect();
		},
		describe: (instance) => instance.GetFullName(),
	};
}
