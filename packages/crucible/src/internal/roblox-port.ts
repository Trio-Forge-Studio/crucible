import type { EnginePort } from "./port";

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
