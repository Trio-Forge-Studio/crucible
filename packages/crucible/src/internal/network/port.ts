import { robloxNetworkPort } from "./roblox-network-port";

/** @internal */
export type Cancel = () => void;

/** @internal */
export interface Discovery {
	readonly className: string;
	readonly protocol: unknown;
	readonly catalog: unknown;
}

/** @internal */
export interface NetworkPort {
	readonly create: (receive: (player: Player, ...args: unknown[]) => void) => void;
	/** Sets the attributes before parenting, so a client that finds the remote can read them (rule 12). */
	readonly publish: (protocol: number, catalog: string) => void;
	readonly fireClient: (player: Player, packet: buffer, sidecar: Readonly<Record<string, Instance>>) => void;
	readonly players: () => readonly Player[];
	readonly onPlayerAdded: (listener: (player: Player) => void) => Cancel;
	readonly onPlayerRemoving: (listener: (player: Player) => void) => Cancel;
	readonly kick: (player: Player, message: string) => void;
	readonly destroy: () => void;

	/** Connects OnClientEvent before returning, so the engine delivers what the server already fired. */
	readonly discover: (timeout: number, receive: (...args: unknown[]) => void) => Discovery | undefined;
	readonly disconnect: () => void;
	readonly fireServer: (packet: buffer, sidecar: Readonly<Record<string, Instance>>) => void;

	readonly delay: (seconds: number, callback: () => void) => Cancel;
	/** `task.spawn(thread, ...)`, which runs the thread to its first yield before returning (rule 52). */
	readonly resume: (thread: thread, ...args: unknown[]) => void;
}

let installed: NetworkPort | undefined;

/** @internal */
export function installNetworkPort(port: NetworkPort) {
	installed = port;
}

/** @internal */
export function networkPort(): NetworkPort {
	return installed ?? robloxNetworkPort();
}
