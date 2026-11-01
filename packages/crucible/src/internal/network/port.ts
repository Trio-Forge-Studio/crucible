import { robloxNetworkPort } from "./roblox-network-port";

/** @internal Ends a subscription or timer. Idempotent. */
export type Cancel = () => void;

/** @internal What client discovery found under ReplicatedStorage (rule 13). */
export interface Discovery {
	readonly className: string;
	readonly protocol: unknown;
	readonly catalog: unknown;
}

/**
 * @internal
 * Everything networking asks of the engine. There is no cancel: networking never cancels an event or request callback
 * (rule 45), and lifecycle's runner cancels a ClientReady callback that yields (rule 34).
 */
export interface NetworkPort {
	/** Server. Creates `ReplicatedStorage.CrucibleNetwork` unparented and connects OnServerEvent. */
	readonly create: (receive: (player: Player, ...args: unknown[]) => void) => void;
	/** Server. Sets the Protocol and Catalog attributes, then parents the remote (rule 12). */
	readonly publish: (protocol: number, catalog: string) => void;
	readonly fireClient: (player: Player, packet: buffer, sidecar: Readonly<Record<string, Instance>>) => void;
	readonly players: () => readonly Player[];
	readonly onPlayerAdded: (listener: (player: Player) => void) => Cancel;
	readonly onPlayerRemoving: (listener: (player: Player) => void) => Cancel;
	readonly kick: (player: Player, message: string) => void;
	/** Server. Disconnects OnServerEvent and destroys the remote. Idempotent. */
	readonly destroy: () => void;

	/**
	 * Client. Waits at most `timeout` seconds for the remote. When it is a RemoteEvent, connects OnClientEvent before
	 * returning, so the engine delivers anything already fired.
	 */
	readonly discover: (timeout: number, receive: (...args: unknown[]) => void) => Discovery | undefined;
	/** Client. Disconnects OnClientEvent. Idempotent. */
	readonly disconnect: () => void;
	readonly fireServer: (packet: buffer, sidecar: Readonly<Record<string, Instance>>) => void;

	/** `task.delay`, returning its cancel. Ask deadlines and handler lifetimes. */
	readonly delay: (seconds: number, callback: () => void) => Cancel;
	/** `task.spawn(thread, ...)`: runs a thread to its first yield before returning (rules 44, 52). */
	readonly resume: (thread: thread, ...args: unknown[]) => void;
}

let installed: NetworkPort | undefined;

/** @internal Test hook. Production code never calls it. */
export function installNetworkPort(port: NetworkPort) {
	installed = port;
}

/** @internal */
export function networkPort(): NetworkPort {
	return installed ?? robloxNetworkPort();
}
