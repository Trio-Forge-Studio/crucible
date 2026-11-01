import { CATALOG_ATTRIBUTE, PROTOCOL_ATTRIBUTE, REMOTE_NAME } from "./limits";
import type { NetworkPort } from "./port";

/** @internal */
export function robloxNetworkPort(): NetworkPort {
	const players = game.GetService("Players");
	const storage = game.GetService("ReplicatedStorage");
	let remote: RemoteEvent | undefined;
	let connection: RBXScriptConnection | undefined;
	function disconnect() {
		connection?.Disconnect();
		connection = undefined;
	}
	return {
		create: (receive) => {
			const created = new Instance("RemoteEvent");
			created.Name = REMOTE_NAME;
			connection = created.OnServerEvent.Connect(receive);
			remote = created;
		},
		publish: (protocol, catalog) => {
			const target = remote ?? error("Crucible network remote was not created");
			target.SetAttribute(PROTOCOL_ATTRIBUTE, protocol);
			target.SetAttribute(CATALOG_ATTRIBUTE, catalog);
			target.Parent = storage;
		},
		fireClient: (player, packet, sidecar) => remote?.FireClient(player, packet, sidecar),
		players: () => players.GetPlayers(),
		onPlayerAdded: (listener) => {
			const c = players.PlayerAdded.Connect(listener);
			return () => c.Disconnect();
		},
		onPlayerRemoving: (listener) => {
			const c = players.PlayerRemoving.Connect(listener);
			return () => c.Disconnect();
		},
		kick: (player, message) => player.Kick(message),
		destroy: () => {
			disconnect();
			remote?.Destroy();
			remote = undefined;
		},
		discover: (timeout, receive) => {
			const found = storage.WaitForChild(REMOTE_NAME, timeout);
			if (found === undefined) return undefined;
			if (found.IsA("RemoteEvent")) {
				remote = found;
				connection = found.OnClientEvent.Connect(receive);
			}
			return {
				className: found.ClassName,
				protocol: found.GetAttribute(PROTOCOL_ATTRIBUTE),
				catalog: found.GetAttribute(CATALOG_ATTRIBUTE),
			};
		},
		disconnect,
		fireServer: (packet, sidecar) => remote?.FireServer(packet, sidecar),
		delay: (seconds, callback) => {
			const thread = task.delay(seconds, callback);
			return () => {
				if (coroutine.status(thread) === "suspended") task.cancel(thread);
			};
		},
		resume: (thread, ...args) => {
			task.spawn(thread, ...args);
		},
	};
}
